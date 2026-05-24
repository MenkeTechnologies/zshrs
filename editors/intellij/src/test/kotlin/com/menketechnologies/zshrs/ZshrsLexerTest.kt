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

    @Test fun `leading-dot identifiers stay one token (zinit-style)`() {
        // zinit defines functions like `.zinit-any-colorify-as-uspl2`.
        // Per zsh's `Src/utils.c::inittyptab`, `.` and `-` are both in
        // the IUSER class so they're valid in function names. Without
        // this support `.zinit-foo` was OPERATOR + IDENTIFIER + OPERATOR
        // + IDENTIFIER and any mid-name `do` / `load` / etc. got mis-
        // classified as a keyword.
        val toks = nonWs(".zinit-load() { :; }")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals(".zinit-load", toks[0].second)
    }

    @Test fun `leading-plus identifiers stay one token (zsh-extension hooks)`() {
        // `+vi-mode-set` is a real zsh-syntax-highlighting / p10k hook name.
        val toks = nonWs("+vi-mode-set foo bar")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("+vi-mode-set", toks[0].second)
    }

    @Test fun `leading-at identifiers stay one token`() {
        // `@hook-fn` style — used by some plugin frameworks.
        val toks = nonWs("@hook-fn arg")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("@hook-fn", toks[0].second)
    }

    @Test fun `bare dot still routes through other paths`() {
        // `.` alone (the `source` builtin) must NOT become a sigil-start
        // — the next char isn't a word char so the leading-sigil path
        // skips. Falls through to operator-char handling.
        val toks = nonWs(". ./foo.zsh")
        // First token is `.` — not an IDENTIFIER bearing `.zsh` etc.
        assertNotEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
    }

    @Test fun `hyphenated identifiers stay one token`() {
        // Regression: `daemon-lock-do` was lexing as IDENTIFIER, OPERATOR,
        // IDENTIFIER, OPERATOR, CONTROL_KEYWORD (`do`) — the IDE then
        // highlighted `-` and `do` differently and treated them as
        // separate symbols. zsh allows hyphens inside function / command
        // names, so the whole thing must be ONE token.
        val toks = nonWs("daemon-lock-do() { :; }")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("daemon-lock-do", toks[0].second)
    }

    @Test fun `hyphenated identifier with builtin-name segment is not a builtin`() {
        // Regression: `daemon-export-pdf` got mis-classified because the
        // lexer split at `-` and `export` matched the BUILTIN/DECL set.
        // Once hyphens are part of the identifier the whole token is
        // looked up against the keyword sets — `daemon-export-pdf` isn't
        // there, so it stays a plain IDENTIFIER.
        val toks = nonWs("daemon-export-pdf alias docs.pdf")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("daemon-export-pdf", toks[0].second)
    }

    @Test fun `trailing hyphen is not absorbed into the identifier`() {
        // `cmd -x` — the `-` starts a separate operator/flag token.
        // The hyphen-extension only kicks in when followed by a word char.
        val toks = nonWs("cmd -x")
        assertEquals("cmd", toks[0].second)
        // Whatever the second token type is (OPERATOR + IDENTIFIER, etc.),
        // it must NOT be `cmd-x`.
        assertNotEquals("cmd-x", toks[0].second)
    }

    @Test fun `double-quoted string`() {
        val toks = nonWs("echo \"hello world\"")
        assertEquals(ZshrsTokenTypes.STRING_DQ, toks[1].first)
        assertEquals("\"hello world\"", toks[1].second)
    }

    @Test fun `double-quoted string with dollar var splits into segments`() {
        // Regression: `"hello $foo world"` used to lex as a single
        // STRING_DQ token, which made `ZshrsQuoteHandler.isInsideLiteral`
        // return true at every cursor position inside the string and
        // suppressed completion / variable-coloring on `$foo`.
        val toks = nonWs("echo \"hello \$foo world\"")
        // toks[0] = `echo`, toks[1..N] = the string segments
        val stringSegments = toks.drop(1)
        val labels = stringSegments.map { it.first to it.second }
        // Must contain at least one VARIABLE / ENV_VAR token for $foo.
        assertTrue(
            "expected VARIABLE / ENV_VAR token for `\$foo` inside string: $labels",
            labels.any { (t, s) ->
                (t == ZshrsTokenTypes.VARIABLE || t == ZshrsTokenTypes.ENV_VAR) && s == "\$foo"
            },
        )
        // The literal segments around the var must also be present.
        assertTrue(
            "missing leading string segment: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.STRING_DQ && s.contains("hello ") },
        )
        assertTrue(
            "missing trailing string segment: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.STRING_DQ && s.contains(" world") },
        )
    }

    @Test fun `double-quoted string with brace expansion splits into segments`() {
        // `"x=${HOME:-/tmp}y"` — must yield STRING_DQ, PARAM_EXPANSION,
        // STRING_DQ so the brace-expansion gets its own color slot and
        // completion fires inside `${…}`.
        val toks = nonWs("echo \"x=\${HOME:-/tmp}y\"")
        val labels = toks.drop(1).map { it.first to it.second }
        assertTrue(
            "expected PARAM_EXPANSION for `\${HOME:-/tmp}` inside string: $labels",
            labels.any { (t, s) ->
                t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${HOME:-/tmp}"
            },
        )
    }

    @Test fun `backtick string with dollar var splits into segments`() {
        // Backticks are the historical command-substitution form. Same
        // interpolation rule as `"..."` — `$var` inside should split out.
        val toks = nonWs("echo `echo \$user`")
        val labels = toks.drop(1).map { it.first to it.second }
        assertTrue(
            "expected VARIABLE / ENV_VAR for `\$user` inside backticks: $labels",
            labels.any { (t, s) ->
                (t == ZshrsTokenTypes.VARIABLE || t == ZshrsTokenTypes.ENV_VAR) && s == "\$user"
            },
        )
    }

    @Test fun `double-quoted string without interpolation stays one token`() {
        // Sanity: when there's no `$`, we don't accidentally split.
        val toks = nonWs("echo \"plain literal\"")
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        assertEquals("split occurred without interpolation: $toks", 1, strings.size)
    }

    @Test fun `escaped dollar inside string does not start interpolation`() {
        // `"cost: \$5"` — the `\$` is an escape, not a var marker. The
        // string should stay one token.
        val toks = nonWs("echo \"cost: \\\$5\"")
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        assertEquals("escaped `\\\$` split the string: $toks", 1, strings.size)
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
