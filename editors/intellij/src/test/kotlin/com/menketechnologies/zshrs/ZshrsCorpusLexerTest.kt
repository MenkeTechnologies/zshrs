package com.menketechnologies.zshrs

import com.intellij.psi.TokenType
import org.junit.Assert.*
import org.junit.Test

/**
 * Corpus-level lexer regression test. Re-lexes every real-world
 * shell file under `examples/` end-to-end and asserts three
 * invariants the user spent multiple turns reporting:
 *
 *   1. Zero BAD_CHARACTER tokens. Any unmatched / un-recognized
 *      char visually flashes red in the IDE — the user's recurring
 *      "syntax highlighting broken" complaint.
 *   2. `${…}` openers balance closers. Off-by-one means the lexer
 *      ate too much / too little and downstream coloring is wrong.
 *   3. Bare `{` / `}` braces balance. Off-by-one means the brace
 *      matcher pairs an inner `{` with the wrong outer `}` — see
 *      `${1:-{\}}` regression report on examples/daemon-shell.zsh.
 *
 * Add files to [CORPUS] when they hit a tricky lexer corner.
 */
class ZshrsCorpusLexerTest {

    @Test fun corpus_examples_daemon_shell_zsh() {
        check("examples/daemon-shell.zsh")
    }

    @Test fun corpus_examples_daemon_shell_sh() {
        check("examples/daemon-shell.sh")
    }

    @Test fun corpus_examples_daemon_shell_bash() {
        check("examples/daemon-shell.bash")
    }

    @Test fun corpus_examples_daemon_shell_ksh() {
        check("examples/daemon-shell.ksh")
    }

    private fun check(rel: String) {
        // Walk up from `gradle.properties` (always in editors/intellij/) to
        // the project root, then resolve the relative path.
        val root = resolveProjectRoot()
        val f = java.io.File(root, rel)
        if (!f.exists()) {
            // Skip rather than fail — some files only exist on local checkouts.
            println("ZshrsCorpusLexerTest: skipping $rel (not found at ${f.path})")
            return
        }
        val src = f.readText()
        val lex = ZshrsLexer()
        lex.start(src, 0, src.length, 0)
        var badCount = 0
        var paramOpeners = 0
        var paramClosers = 0
        var bareLBrace = 0
        var bareRBrace = 0
        val badSamples = mutableListOf<String>()
        while (lex.tokenType != null) {
            val t = lex.tokenType
            val text = src.substring(lex.tokenStart, lex.tokenEnd)
            when {
                t == TokenType.BAD_CHARACTER -> {
                    badCount++
                    if (badSamples.size < 8) {
                        val line = src.substring(0, lex.tokenStart).count { it == '\n' } + 1
                        val col = lex.tokenStart - (src.substring(0, lex.tokenStart).lastIndexOf('\n') + 1)
                        badSamples.add("line $line col $col: [$text]")
                    }
                }
                t == ZshrsTokenTypes.PARAM_EXPANSION && text == "\${" -> paramOpeners++
                t == ZshrsTokenTypes.PARAM_EXPANSION && text == "}" -> paramClosers++
                t == ZshrsTokenTypes.LBRACE -> bareLBrace++
                t == ZshrsTokenTypes.RBRACE -> bareRBrace++
            }
            lex.advance()
        }
        assertEquals(
            "$rel: BAD_CHARACTER tokens emitted at: $badSamples",
            0, badCount,
        )
        assertEquals(
            "$rel: PARAM_EXPANSION openers ($paramOpeners) != closers ($paramClosers)",
            paramOpeners, paramClosers,
        )
        assertEquals(
            "$rel: bare LBRACE ($bareLBrace) != bare RBRACE ($bareRBrace)",
            bareLBrace, bareRBrace,
        )
    }

    /**
     * Resolve the project root by walking up from the test classpath
     * to the `editors/intellij/` parent. Works both in local gradle
     * runs (cwd = editors/intellij) and in CI (cwd = repo root).
     */
    private fun resolveProjectRoot(): java.io.File {
        var d: java.io.File? = java.io.File("").absoluteFile
        repeat(6) {
            if (d == null) return java.io.File(".").absoluteFile
            val candidate = java.io.File(d, "Cargo.toml")
            if (candidate.exists()) return d!!
            d = d?.parentFile
        }
        // Last-resort fallback: current dir.
        return java.io.File(".").absoluteFile
    }
}
