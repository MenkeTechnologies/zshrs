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

    @Test fun `parameter expansion brace block is segmented`() {
        // Pre-2026-05 the whole `${HOME:-/tmp}` lexed as one opaque
        // PARAM_EXPANSION token. New shape splits it so completion can
        // fire on the variable name inside: `${` (PARAM_EXPANSION) +
        // `HOME` (IDENTIFIER / OPTION_NAME) + modifier tokens + `}`
        // (PARAM_EXPANSION).
        val toks = nonWs("\${HOME:-/tmp}")
        val labels = toks.map { it.first to it.second }
        assertTrue(
            "missing PARAM_EXPANSION `\${`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        assertTrue(
            "missing PARAM_EXPANSION `}`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
        // Name is standalone (any identifier-shaped token kind).
        assertTrue(
            "expected `HOME` as its own token: $labels",
            labels.any { (_, s) -> s == "HOME" },
        )
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

    @Test fun `leading-hyphen identifiers stay one token (zsh-syntax-highlighting)`() {
        // `-hsmw-add-highlight` and `-zui_std_cleanup` are real
        // function names from `zsh-syntax-highlighting` and the `zui`
        // framework. Audited against ~/.zinit/plugins/**.
        val toks = nonWs("-hsmw-add-highlight foo bar")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("-hsmw-add-highlight", toks[0].second)
    }

    @Test fun `dot-inside-identifier stays one token (_dotdrop_sh)`() {
        // `_dotdrop.sh-compare` — found in zinit plugins.
        val toks = nonWs("_dotdrop.sh-compare profile")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals("_dotdrop.sh-compare", toks[0].second)
    }

    @Test fun `colon-in-identifier-body stays one token (hist precmd)`() {
        // `:hist:precmd` — async pseudo-namespace pattern.
        val toks = nonWs(":hist:precmd")
        assertEquals(ZshrsTokenTypes.IDENTIFIER, toks[0].first)
        assertEquals(":hist:precmd", toks[0].second)
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
        // Segmented now (opener + body) — concatenate to verify.
        val toks = nonWs("echo \"hello world\"")
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        assertEquals("\"hello world\"", strings.joinToString("") { it.second })
    }

    @Test fun `printf format specs in dq string emit STRING_FORMAT tokens`() {
        // Mirror of strykelang's STRING_FORMAT handling. Inside a
        // `"..."` body, each `%d` / `%s` / `%10.2f` / `%%` etc gets
        // its OWN token type so the IDE can color it distinctly
        // from the surrounding literal text.
        val toks = nonWs("printf \"%d %s %10.2f\\n\" 42 hi 3.14")
        val formats = toks.filter { it.first == ZshrsTokenTypes.STRING_FORMAT }
        val labels = formats.map { it.second }
        assertTrue(
            "expected %d in format tokens: $labels",
            labels.any { it == "%d" },
        )
        assertTrue(
            "expected %s in format tokens: $labels",
            labels.any { it == "%s" },
        )
        assertTrue(
            "expected %10.2f in format tokens: $labels",
            labels.any { it == "%10.2f" },
        )
    }

    @Test fun `double-percent emits one STRING_FORMAT token`() {
        // `%%` is the printf literal-percent escape. Highlight as a
        // single format token (2 chars) so it stands out from prose.
        val toks = nonWs("printf \"50%% done\"")
        val formats = toks.filter { it.first == ZshrsTokenTypes.STRING_FORMAT }
        assertEquals(
            "expected one %% token, got ${formats.map { it.second }}",
            1,
            formats.size,
        )
        assertEquals("%%", formats[0].second)
    }

    @Test fun `percent followed by non-conv-char is literal not a format spec`() {
        // `%` followed by chars that can never end in a conversion
        // letter is literal text. Pick a sentinel that's NOT in any
        // FORMAT_FLAGS/FORMAT_LENGTH/FORMAT_CONV set — `,` is safe
        // (and stays safe under prompt-expansion-code additions like
        // 84e710941b, which added `!` `?` `~` `_` `*` `@` `/` to
        // FORMAT_CONV for zsh prompt-expansion highlighting and so
        // `%!` etc. now DO emit STRING_FORMAT, leaving `,` as the
        // canonical non-conv sentinel for this regression test).
        val toks = nonWs("echo \"go %, away\"")
        val formats = toks.filter { it.first == ZshrsTokenTypes.STRING_FORMAT }
        assertEquals(
            "lone % followed by `,` should NOT be a format spec: ${formats.map { it.second }}",
            0,
            formats.size,
        )
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
        // `"x=${HOME:-/tmp}y"` lexes as:
        //   STRING_DQ "x=  + PARAM_EXPANSION ${  + name+modifier tokens
        //   + PARAM_EXPANSION }  + STRING_DQ y"
        // The PARAM_EXPANSION `${` and `}` brackets MUST exist as
        // standalone tokens so completion fires inside, AND the string
        // must close cleanly without the modifier text leaking into a
        // string segment.
        val toks = nonWs("echo \"x=\${HOME:-/tmp}y\"")
        val labels = toks.map { it.first to it.second }
        assertTrue(
            "missing PARAM_EXPANSION `\${`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        assertTrue(
            "missing PARAM_EXPANSION `}`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
        assertTrue(
            "expected `HOME` as its own token inside `\${…}`: $labels",
            labels.any { (_, s) -> s == "HOME" },
        )
        // String must close — the LAST emitted STRING_DQ should
        // contain the trailing `y"`.
        val lastString = labels.lastOrNull { it.first == ZshrsTokenTypes.STRING_DQ }
        assertNotNull("no closing STRING_DQ segment: $labels", lastString)
        assertTrue(
            "string didn't close cleanly: $lastString",
            lastString!!.second.endsWith("\""),
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

    @Test fun `double-quoted string without interpolation reassembles`() {
        // STRING_DQ segments must concatenate back to the original
        // string (the opener `"` is its own token per the IntelliJ
        // continuity contract — gaps in the token stream crash the
        // searchable-options indexer).
        val toks = nonWs("echo \"plain literal\"")
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        assertEquals(
            "STRING_DQ tokens must reassemble to the original",
            "\"plain literal\"",
            strings.joinToString("") { it.second },
        )
    }

    @Test fun `escaped-brace default value inside string lexes cleanly`() {
        // Regression: `local body="${1:-{\}}"` from daemon-shell.zsh.
        // `${1:-{\}}` is a parameter expansion whose default value
        // is the literal `{}` (the `\}` escapes the inner `}`). The
        // critical invariant: the LSP must end at a STRING_DQ token
        // containing the closing `"` — the param expansion must NOT
        // eat past the string's close quote.
        val toks = nonWs("local body=\"\${1:-{\\}}\"")
        val labels = toks.map { it.first to it.second }
        // The whole `${…}` is now segmented into [`${`, body…, `}`] so
        // completion can fire on the body. Verify the opener and
        // closer are both PARAM_EXPANSION.
        assertTrue(
            "expected PARAM_EXPANSION `\${` opener: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        assertTrue(
            "expected PARAM_EXPANSION `}` closer: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
        // The string must close cleanly — the LAST emitted STRING_DQ
        // segment should end with `"`. Anything past it means the
        // string color is leaking.
        val lastString = toks.lastOrNull { it.first == ZshrsTokenTypes.STRING_DQ }
        assertNotNull("no closing STRING_DQ segment: $labels", lastString)
        assertTrue(
            "closing segment doesn't end with quote: $lastString",
            lastString!!.second.endsWith("\""),
        )
    }

    @Test fun `parameter expansion splits so completion fires on the name`() {
        // Regression: `${HOME}` used to lex as ONE opaque
        // PARAM_EXPANSION token. The IDE couldn't offer completion on
        // the name inside because Cmd-Space would have replaced the
        // entire `${HOME}` with the chosen candidate. Splitting into
        // [`${`, `HOME`, `}`] gives `HOME` its own identifier-shaped
        // token so completion can fire and replace just the name.
        val toks = nonWs("echo \${HOME}")
        val labels = toks.map { it.first to it.second }
        // Opener
        assertTrue(
            "missing PARAM_EXPANSION `\${`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        // Name as a standalone token (any of IDENTIFIER / ENV_VAR /
        // BUILTIN / VARIABLE — all are completable from the IDE's
        // perspective; we don't gate on exact kind, just on
        // standalone-token-ness).
        assertTrue(
            "expected `HOME` as its own token (completable): $labels",
            labels.any { (_, s) -> s == "HOME" },
        )
        // Closer
        assertTrue(
            "missing PARAM_EXPANSION `}`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
    }

    @Test fun `command-group brace still lexes as RBRACE`() {
        // After adding `${…}`-close recognition, verify a plain
        // command-group close (`{ cmds; }`) STILL lexes as the bare
        // LBRACE / RBRACE pair, not as PARAM_EXPANSION. The
        // `paramBraceDepth` tracker scopes the special-casing.
        val toks = nonWs("foo() { :; }")
        val labels = toks.map { it.first to it.second }
        assertTrue(
            "command-group `}` mis-classified as param-close: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.RBRACE && s == "}" },
        )
        assertFalse(
            "command-group `}` leaked into PARAM_EXPANSION: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
    }

    @Test fun `escaped dollar inside string does not start interpolation`() {
        // `"cost: \$5"` — the `\$` is an escape, not a var marker. No
        // VARIABLE / ENV_VAR token should appear; STRING_DQ segments
        // concatenate back to the original.
        val toks = nonWs("echo \"cost: \\\$5\"")
        val labels = toks.map { it.first to it.second }
        assertFalse(
            "escaped `\\\$` mis-classified as interpolation: $labels",
            labels.any { (t, _) ->
                t == ZshrsTokenTypes.VARIABLE || t == ZshrsTokenTypes.ENV_VAR
            },
        )
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        assertEquals(
            "STRING_DQ segments must reassemble to the original",
            "\"cost: \\\$5\"",
            strings.joinToString("") { it.second },
        )
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

    @Test fun `hash inside param-length expansion is not a comment`() {
        // Regression from screenshot: `${#tags[@]}` was getting the `#`
        // colored as a comment-start. The whole `${…}` is a parameter-
        // length expansion — `#` inside `${` is the length operator,
        // not `#`-comment. Verify the entire span lexes as one
        // PARAM_EXPANSION segmented token (`${` + `#tags[@]` + `}`)
        // and the `#` is NEVER emitted as COMMENT.
        val toks = nonWs("if (( \${#tags[@]} > 0 ))")
        val labels = toks.map { it.first to it.second }
        assertFalse(
            "`#` inside `\${#…}` got comment-colored: $labels",
            labels.any { (t, _) -> t == ZshrsTokenTypes.COMMENT },
        )
        // The `${` opener must be present (segmented param-expansion).
        assertTrue(
            "missing PARAM_EXPANSION `\${` opener: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
    }

    @Test fun `daemon-shell two-string concatenation lexes both strings cleanly`() {
        // Exact line from daemon-shell.zsh that the user flagged:
        //   local esc="${t//\\/\\\\}"; esc="${esc//\"/\\\"}"
        // Two `local`-assigned strings, both with `${…}` substitutions
        // containing escapes. Must lex as: local, esc, =, "..", ;,
        //                     esc, =, ".." — no merged or split tokens.
        val src = "local esc=\"\${t//\\\\/\\\\\\\\}\"; esc=\"\${esc//\\\"/\\\\\\\"}\""
        val toks = nonWs(src)
        val labels = toks.map { it.first to it.second }
        // Count PARAM_EXPANSION `${` openers — must be exactly 2.
        val openers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" }
        assertEquals("expected 2 `\${` openers (one per substitution): $labels", 2, openers)
        // Count PARAM_EXPANSION `}` closers — must be exactly 2.
        val closers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" }
        assertEquals("expected 2 `}` closers: $labels", 2, closers)
        // No COMMENT tokens — none of the `#` chars appear here so
        // this also pins the `${#…}` regression by adjacency.
        assertFalse(
            "no `#` in source but lexer emitted COMMENT anyway: $labels",
            labels.any { (t, _) -> t == ZshrsTokenTypes.COMMENT },
        )
    }

    @Test fun `escaped quote inside param expansion does not break string`() {
        // Regression from screenshot: `esc="${esc//\"/\\\"}"` was
        // getting lexed wrong — the `\"` inside the `${…}` substitution
        // was being treated as the closing quote of the outer string.
        // The `\"` inside `${…}` is part of the substitution pattern,
        // not a string terminator. Verify: outer string closes on the
        // FINAL `"`, not a mid-pattern `\"`.
        val src = "esc=\"\${esc//\\\"/\\\\\\\"}\""
        val toks = nonWs(src)
        val labels = toks.map { it.first to it.second }
        // PARAM_EXPANSION opener + closer must both appear.
        assertTrue(
            "missing PARAM_EXPANSION `\${`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        assertTrue(
            "missing PARAM_EXPANSION `}`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" },
        )
        // The LAST STRING_DQ segment must end at the outermost `"`.
        val lastString = labels.lastOrNull { it.first == ZshrsTokenTypes.STRING_DQ }
        assertNotNull("no closing STRING_DQ segment: $labels", lastString)
        assertTrue(
            "outer string closed at wrong `\"`: $lastString",
            lastString!!.second.endsWith("\""),
        )
    }

    @Test fun `daemon-shell _daemon_post full function lexes cleanly`() {
        // Full reported context — the `${1:-{\}}` paramBraceDepth must
        // RESET to 0 after the `}` so the next line's `\` continuation,
        // string content, etc. don't bleed under "in-param" handling.
        // If `paramBraceDepth` leaks, subsequent `}` chars get mis-
        // emitted as PARAM_EXPANSION closers instead of bare RBRACE,
        // and the function's own closing `}` gets the wrong color.
        val src = """
            # Internal: POST to /op/<name> with JSON body. First arg = op name.
            # Remaining args interpreted as raw JSON object body (single string).
            _daemon_post() {
                local op="${'$'}1"; shift
                local body="${'$'}{1:-{\}}"
                _daemon_curl \
                    -H 'Content-Type: application/json' \
                    --data-raw "${'$'}body" \
                    "${'$'}DAEMON_URL/op/${'$'}op"
            }
        """.trimIndent() + "\n"
        val toks = tokens(src)
            .filter { it.first != com.intellij.psi.TokenType.WHITE_SPACE }
        val labels = toks.map { it.first to it.second }
        assertFalse(
            "BAD_CHARACTER emitted: ${labels.filter { it.first == com.intellij.psi.TokenType.BAD_CHARACTER }}",
            labels.any { (t, _) -> t == com.intellij.psi.TokenType.BAD_CHARACTER },
        )
        // The `${1:-{\}}` block opens + closes once. The function's
        // outer `{` and `}` are bare braces, NOT param-expansion
        // closers. Total `${` openers across the whole snippet: 1.
        val openers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" }
        assertEquals(
            "expected exactly 1 `\${` opener (only `\${1:-{\\}}` on line 5): $labels",
            1, openers,
        )
        // PARAM_EXPANSION closers: 1 (matching the one opener above).
        // The function's outer `}` on the last line is a bare RBRACE,
        // NOT a param-close — if `paramBraceDepth` leaks the count
        // jumps to 2 and the function's `}` gets the wrong color.
        val paramClosers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" }
        assertEquals(
            "expected exactly 1 PARAM_EXPANSION `}` (function close must be bare RBRACE): $labels",
            1, paramClosers,
        )
        // The function's outer `{` and `}` are bare braces.
        val lbraces = labels.count { (t, s) -> t == ZshrsTokenTypes.LBRACE && s == "{" }
        val rbraces = labels.count { (t, s) -> t == ZshrsTokenTypes.RBRACE && s == "}" }
        assertEquals("missing bare `{` for function body: $labels", 1, lbraces)
        assertEquals("missing bare `}` for function close: $labels", 1, rbraces)
        // No comment leakage past the actual `#` comment lines.
        // Both `# Internal:` and `# Remaining args` are valid comments;
        // count them — must be exactly 2.
        val comments = labels.count { (t, _) -> t == ZshrsTokenTypes.COMMENT }
        assertEquals("expected exactly 2 comment lines: $labels", 2, comments)
    }

    @Test fun `daemon-shell local body line dumps cleanly`() {
        // Dump every token for the exact reported line so any
        // regression in this input pattern surfaces with a readable
        // diff. Pinning the assertions on STRING_DQ contiguity (no
        // mid-line color flips into COMMENT / BAD_CHARACTER / OPERATOR
        // / etc.) is what the user actually sees as "broken
        // highlighting".
        val src = "    local body=\"\${1:-{\\}}\""
        val toks = tokens(src)
            .filter { it.first != com.intellij.psi.TokenType.WHITE_SPACE }
        val labels = toks.map { it.first to it.second }
        // No bad chars.
        assertFalse(
            "BAD_CHARACTER emitted: $labels",
            labels.any { (t, _) -> t == com.intellij.psi.TokenType.BAD_CHARACTER },
        )
        // No spurious COMMENT (the `#` inside `${#…}` regression also
        // hit `${…}` defaults with braces — pin both).
        assertFalse(
            "COMMENT leaked into string: $labels",
            labels.any { (t, _) -> t == ZshrsTokenTypes.COMMENT },
        )
        // Strings reassemble — opening `"` + tail `"` segments cover
        // the literal text NOT inside the `${…}`.
        val strings = toks.filter { it.first == ZshrsTokenTypes.STRING_DQ }
        val reassembled = strings.joinToString("") { it.second }
        // Both quotes survived (one in opening segment, one in closing
        // segment — they may be in the same or separate tokens).
        assertEquals(
            "string segments must reassemble to `\"\"` (the literal text outside `\${…}`): $labels",
            "\"\"",
            reassembled,
        )
        // Exactly one PARAM_EXPANSION `${` opener and one `}` closer.
        val openers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" }
        assertEquals("opener count wrong: $labels", 1, openers)
        val closers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" }
        assertEquals("closer count wrong: $labels", 1, closers)
    }

    @Test fun `escaped brace inside parameter default closes correctly`() {
        // Pin: `${1:-{\}}` (zsh idiom for defaulting to literal `{}`)
        // — outer `${` opens, inner `{` is literal in the default
        // value, `\}` escapes a literal `}`, and the FIRST plain `}`
        // closes the param expansion. The whole thing is segmented
        // into [`${`, body…, `}`] so completion fires on `1`.
        val toks = nonWs("\${1:-{\\}}")
        val labels = toks.map { it.first to it.second }
        assertTrue(
            "missing PARAM_EXPANSION `\${`: $labels",
            labels.any { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "\${" },
        )
        // Exactly ONE closing `}` — anything more means the brace
        // counter mis-matched the escape and ate too much.
        val closers = labels.count { (t, s) -> t == ZshrsTokenTypes.PARAM_EXPANSION && s == "}" }
        assertEquals("must have exactly one closer: $labels", 1, closers)
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

    // ── DOC_COMMENT (## doubled-hash) vs regular COMMENT (#) ────────

    @Test
    fun double_hash_emits_doc_comment_token() {
        val toks = nonWs("## doc comment line\n# regular comment line\n")
        val docs = toks.filter { it.first == ZshrsTokenTypes.DOC_COMMENT }
        val regs = toks.filter { it.first == ZshrsTokenTypes.COMMENT }
        assertTrue("expected DOC_COMMENT for `## …`: $toks", docs.isNotEmpty())
        assertTrue("expected COMMENT for `# …`: $toks", regs.isNotEmpty())
        assertTrue("doc text mismatch: ${docs[0]}", docs[0].second.startsWith("## "))
        assertTrue("reg text mismatch: ${regs[0]}", regs[0].second.startsWith("# "))
    }

    @Test
    fun single_hash_alone_stays_regular_comment() {
        // `#` with no trailing content is still a regular comment.
        val toks = nonWs("#\nfoo\n")
        val c = toks.first {
            it.first == ZshrsTokenTypes.COMMENT || it.first == ZshrsTokenTypes.DOC_COMMENT
        }
        assertEquals(ZshrsTokenTypes.COMMENT, c.first)
    }

    @Test
    fun shebang_remains_distinct_from_doc_comment() {
        // `#!` stays SHEBANG; the doubled-hash on the next line
        // still becomes DOC_COMMENT independently.
        val toks = nonWs("#!/usr/bin/env zsh\n## then a doc comment\n")
        assertTrue(
            "expected SHEBANG on line 1: $toks",
            toks.any { it.first == ZshrsTokenTypes.SHEBANG },
        )
        assertTrue(
            "expected DOC_COMMENT on line 2: $toks",
            toks.any { it.first == ZshrsTokenTypes.DOC_COMMENT },
        )
    }

    // ── Long-flag (--word) recognition ─────────────────────────────

    @Test
    fun long_flag_is_single_identifier_token_not_two_dashes() {
        // `zshrs --version` previously lexed as IDENTIFIER + OPERATOR
        // + OPERATOR + IDENTIFIER, which painted `--` red/badly.
        // Now `--version` is a single IDENTIFIER spanning all 9 chars.
        val toks = nonWs("zshrs --version")
        val longFlag = toks.find { it.second == "--version" }
        assertNotNull("`--version` token missing: $toks", longFlag)
        assertEquals(
            "expected `--version` as single IDENTIFIER",
            ZshrsTokenTypes.IDENTIFIER as IElementType,
            longFlag!!.first as IElementType,
        )
        // Sanity — no stray OPERATOR `-` tokens.
        assertTrue(
            "must not split `--` into two OPERATOR tokens: $toks",
            toks.none { it.first == ZshrsTokenTypes.OPERATOR && it.second == "-" },
        )
    }

    @Test
    fun long_flag_with_hyphen_inside_word_stays_one_token() {
        // `--gen-docs`, `--dump-reflection`, `--no-rcs` — hyphens
        // INSIDE the long-flag name don't terminate the token.
        for (flag in listOf("--gen-docs", "--dump-reflection", "--no-rcs")) {
            val toks = nonWs("zshrs $flag")
            val t = toks.find { it.second == flag }
            assertNotNull("token `$flag` missing: $toks", t)
            assertEquals(
                "expected `$flag` as single IDENTIFIER",
                ZshrsTokenTypes.IDENTIFIER as IElementType,
                t!!.first as IElementType,
            )
        }
    }

    @Test
    fun bare_double_dash_separator_keeps_old_operator_lex() {
        // `--` alone (not followed by a word char) is the
        // positional-args separator. NOT a long-flag; falls back
        // to the operator path.
        val toks = nonWs("foo -- bar")
        // Should NOT have a single `--` IDENTIFIER; the two dashes
        // remain operator tokens.
        assertTrue(
            "bare `--` should not lex as IDENTIFIER: $toks",
            toks.none { it.first == ZshrsTokenTypes.IDENTIFIER && it.second == "--" },
        )
    }

    @Test
    fun arithmetic_inside_dq_string_emits_opener_then_interior_atoms() {
        // `"$(( 7 % 3 ))"` — the arithmetic interior must NOT be
        // re-absorbed into the STRING_DQ token (which was the user-
        // visible italic-green coloring bug). After the strykelang-
        // style state-machine fix, we expect to see:
        //   * STRING_DQ "\""            (opening quote)
        //   * OPERATOR "\$(("           (3-byte arithmetic opener)
        //   * INTEGER  "7"              (sub-lex inside DQ_ARITH)
        //   * OPERATOR "%"              (operator atom)
        //   * INTEGER  "3"              (operand atom)
        //   * OPERATOR "))"             (2-byte closer)
        //   * STRING_DQ "\""            (closing quote)
        val toks = nonWs("echo \"\$(( 7 % 3 ))\"")
        // Verify the OPERATOR `$((` and OPERATOR `))` are present.
        assertTrue(
            "expected OPERATOR `\$((` opener in: $toks",
            toks.any { it.first == ZshrsTokenTypes.OPERATOR && it.second == "\$((" },
        )
        assertTrue(
            "expected OPERATOR `))` closer in: $toks",
            toks.any { it.first == ZshrsTokenTypes.OPERATOR && it.second == "))" },
        )
        // Verify the digit `7` lexed as its own token (not absorbed
        // into a STRING_DQ run).
        assertTrue(
            "expected digit `7` as a separate token in: $toks",
            toks.any { it.second == "7" && it.first != ZshrsTokenTypes.STRING_DQ },
        )
        // No STRING_DQ should contain the arith body text.
        assertTrue(
            "STRING_DQ must not absorb arith body in: $toks",
            toks.none {
                it.first == ZshrsTokenTypes.STRING_DQ
                && (it.second.contains("7 % 3") || it.second.contains("\$(("))
            },
        )
    }

    @Test
    fun cmdsub_inside_dq_string_emits_opener_then_interior_atoms() {
        // `"$( echo hi )"` — same shape as the arith fix but the
        // closer is a single `)` instead of `))`. Pin the OPERATOR
        // `$(` opener + closer + interior `echo` token.
        val toks = nonWs("x=\"\$( echo hi )\"")
        assertTrue(
            "expected OPERATOR `\$(` opener: $toks",
            toks.any { it.first == ZshrsTokenTypes.OPERATOR && it.second == "\$(" },
        )
        assertTrue(
            "expected OPERATOR `)` closer: $toks",
            toks.any { it.first == ZshrsTokenTypes.OPERATOR && it.second == ")" },
        )
        // `echo` should be its own non-STRING token.
        assertTrue(
            "expected `echo` not inside STRING_DQ: $toks",
            toks.any { it.second == "echo" && it.first != ZshrsTokenTypes.STRING_DQ },
        )
    }
}
