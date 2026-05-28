package com.menketechnologies.zshrs

import com.intellij.codeInsight.editorActions.smartEnter.SmartEnterProcessor
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.ScrollType
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiFile

/**
 * Complete Current Statement (Cmd-Shift-Enter) for zsh source.
 *
 * Strategy chain, tried in priority order:
 *
 *  1. **Control-flow block** — `if` / `elif` / `while` / `until` /
 *     `for` / `foreach` / `select` / `repeat` / `case` / `else` /
 *     `do` / `then`. Inserts the matching body opener (`then` / `do`
 *     / `in`) and closer (`fi` / `done` / `esac`) on new lines and
 *     drops the caret in the body. zsh body grammar is keyword-bracketed
 *     (`if ... fi`, `while ... done`, `case ... esac`), so we can't
 *     reuse the C-style `{ ... }` shape from other languages.
 *
 *  2. **Function declaration** — `function foo` / `function foo()` /
 *     `foo()`. Appends ` {\n    |\n}` and parks the caret on the body
 *     line. Skips when the body brace already follows on this or the
 *     next line.
 *
 *  3. **Bracket / quote balance** — unclosed `[[` / `((` / `(` / `[`
 *     / `{` / `"` / `'` on the current line. Closing tokens are
 *     appended at end of line (skipping the comment region) and the
 *     caret lands just after them. zsh-specific: the 2-char openers
 *     `[[` and `((` must be detected before the single-bracket logic
 *     so they close as `]]` / `))` instead of `]` + `]`.
 *
 *  4. **Bare body opener** — line ends with `then` / `do` / `in`.
 *     Just add a newline + body indent so the user can keep typing.
 *
 * Skipped (return false → platform default Enter): comment lines,
 * lines where the matching body opener / closer is already present,
 * and anything else we can't structurally recognise.
 *
 * Pure-function planner lives in [Companion.computePlan] so the test
 * suite can exercise every strategy without a platform fixture — see
 * [ZshrsSmartEnterProcessorTest].
 */
class ZshrsSmartEnterProcessor : SmartEnterProcessor() {
    override fun process(project: Project, editor: Editor, file: PsiFile): Boolean {
        if (file.fileType !is ZshrsFileType) return false

        val doc = editor.document
        val caret = editor.caretModel.offset
        val text = doc.charsSequence

        val lineNum = doc.getLineNumber(caret)
        val lineStart = doc.getLineStartOffset(lineNum)
        val lineEnd = doc.getLineEndOffset(lineNum)
        val line = text.subSequence(lineStart, lineEnd).toString()

        val plan = computePlan(line, lineStart, caret, text) ?: return false

        // Platform's SmartEnter action wraps `process` in a write
        // command (mirrors XmlSmartEnterProcessor / JsonSmartEnter-
        // Processor in intellij-community), so direct doc mutation
        // is safe here.
        doc.insertString(plan.offset, plan.insert)
        commit(editor)
        editor.caretModel.moveToOffset(plan.offset + plan.caretRel)
        editor.scrollingModel.scrollToCaret(ScrollType.RELATIVE)
        return true
    }

    companion object {
        /** Computed edit: insert [insert] at [offset], caret to [offset]+[caretRel]. */
        data class Plan(val offset: Int, val insert: String, val caretRel: Int)

        /**
         * Pure function: given a zsh source line, return the edit plan
         * that completes its statement, or `null` if no strategy
         * matches. Offsets are absolute (`lineStart`-based) so the
         * caller can apply directly to the document.
         */
        fun computePlan(line: String, lineStart: Int, caret: Int, text: CharSequence): Plan? {
            val trimmed = line.trimStart()
            if (trimmed.startsWith("#")) return null

            tryControlBlock(line, lineStart, text)?.let { return it }
            tryFunctionDecl(line, lineStart, text)?.let { return it }
            tryBracketBalance(line, lineStart)?.let { return it }
            tryBareBodyOpener(line, lineStart, text)?.let { return it }
            return null
        }

        // ── Strategy 1: control-flow blocks ──────────────────────

        private fun tryControlBlock(line: String, lineStart: Int, text: CharSequence): Plan? {
            val trimmed = line.trimStart()
            val kw = CONTROL_KEYWORD.matchAt(trimmed, 0) ?: return null
            val keyword = kw.value
            val indent = leadingIndent(line)

            // `else` — no header, no closer, just newline + indent.
            // The enclosing `fi` is the user's responsibility.
            if (keyword == "else") {
                if (lineEndsWith(line, "else")) {
                    val afterLine = lineStart + line.trimEnd().length
                    val body = "\n$indent    "
                    return Plan(afterLine, body, body.length)
                }
                return null
            }

            // `then` / `do` alone at end of line — bare body opener:
            // user typed e.g. `if [[ $x ]]; then`. Add a newline.
            // Handled by tryBareBodyOpener; skip here so we don't
            // double-fire.
            if (keyword in BODY_OPENER_WORDS) return null

            // Already has matching opener (`if ... then`, `for ... do`,
            // `case ... in`) on this line — likely the user pressed
            // smart-enter at end of header just to insert the body
            // (no header completion needed).
            val opener = BODY_OPENERS[keyword] ?: return null
            val closer = BODY_CLOSERS[keyword]    // may be null for `elif`

            val openerOnLine = containsKeywordOutsideStrings(line, opener)

            // `case X in` — `in` is the body opener already part of
            // the header. Append `\n    |\nesac` if it's there, else
            // require user to type `in` first.
            if (keyword == "case") {
                val esac = closer ?: return null
                if (!openerOnLine) {
                    // `case $x|` — append ` in\n    |\nesac`.
                    if (lineEndsWithBareToken(line)) {
                        val afterLine = lineStart + line.trimEnd().length
                        val insert = " in\n$indent    \n$indent$esac"
                        val caretRel = 4 + indent.length + 4  // skip " in\n" + indent + 4 body indent
                        return Plan(afterLine, insert, caretRel)
                    }
                    return null
                }
                // `case $x in|` — append body + esac.
                if (followingKeywordPresent(text, lineStart, line, esac)) return null
                val afterLine = lineStart + line.trimEnd().length
                val insert = "\n$indent    \n$indent$esac"
                val caretRel = 1 + indent.length + 4
                return Plan(afterLine, insert, caretRel)
            }

            // `elif` — header completes with `; then\n    |` but NO
            // closer (the enclosing `if`'s `fi` covers it).
            if (keyword == "elif") {
                if (openerOnLine) return null
                if (lineHasUnclosedBracket(line)) return null
                val afterLine = lineStart + line.trimEnd().length
                val insert = "; then\n$indent    "
                val caretRel = insert.length
                return Plan(afterLine, insert, caretRel)
            }

            // `if` / `while` / `until` / `for` / `foreach` / `select`
            // / `repeat`. Header → `; OPENER\n<body>\nCLOSER`.
            if (openerOnLine) {
                // Body opener already present — caret in body, just
                // ensure closer follows. If closer is on a later line
                // we're done; if absent, append.
                if (followingKeywordPresent(text, lineStart, line, closer ?: return null)) return null
                val afterLine = lineStart + line.trimEnd().length
                val insert = "\n$indent    \n$indent$closer"
                val caretRel = 1 + indent.length + 4
                return Plan(afterLine, insert, caretRel)
            }

            // Don't slam a `; then` on an unbalanced `[[`/`((`/`(`.
            if (lineHasUnclosedBracket(line)) return null
            if (followingKeywordPresent(text, lineStart, line, closer ?: return null)) return null

            val afterLine = lineStart + line.trimEnd().length
            val insert = "; $opener\n$indent    \n$indent$closer"
            // Offset breakdown: "; $opener\n" + indent + body 4 spaces.
            val caretRel = 2 + opener.length + 1 + indent.length + 4
            return Plan(afterLine, insert, caretRel)
        }

        // ── Strategy 2: function declarations ────────────────────

        private fun tryFunctionDecl(line: String, lineStart: Int, text: CharSequence): Plan? {
            val trimmed = line.trimStart()
            val indent = leadingIndent(line)

            // `function NAME` / `function NAME()` — `function`
            // keyword followed by an identifier.
            val funcKw = FUNCTION_DECL.matchAt(trimmed, 0)
            if (funcKw != null) {
                // If body already started on this or next line, bail.
                if (lineHasOpenBrace(line)) return null
                if (hasOpenBraceAfter(text, lineStart + line.trimEnd().length)) return null
                val needsParens = !trimmed.substring(funcKw.range.last + 1)
                    .trimEnd()
                    .endsWith(")")
                val afterLine = lineStart + line.trimEnd().length
                val parens = if (needsParens) "()" else ""
                val insert = "$parens {\n$indent    \n$indent}"
                val caretRel = parens.length + 3 + indent.length + 4
                return Plan(afterLine, insert, caretRel)
            }

            // POSIX form: `NAME()` at start of line, no body brace.
            // Strict guard: the line must be just IDENT `()` (with
            // optional surrounding whitespace) — anything else (e.g.
            // `foo() { … }`) is already complete or is a call, not
            // a decl.
            val posix = POSIX_FUNC_DECL.matchEntire(line.trim())
            if (posix != null) {
                if (lineHasOpenBrace(line)) return null
                if (hasOpenBraceAfter(text, lineStart + line.trimEnd().length)) return null
                val afterLine = lineStart + line.trimEnd().length
                val insert = " {\n$indent    \n$indent}"
                val caretRel = 3 + indent.length + 4
                return Plan(afterLine, insert, caretRel)
            }
            return null
        }

        // ── Strategy 3: bracket / quote balance ──────────────────

        private fun tryBracketBalance(line: String, lineStart: Int): Plan? {
            // Track 2-char `[[` and `((` along with the single-char
            // brackets. Quote regions skipped so `"hello (world"`
            // doesn't register the inner `(` as unclosed.
            val stack = ArrayDeque<String>()
            var i = 0
            while (i < line.length) {
                val c = line[i]
                val pair = if (i + 1 < line.length) "" + c + line[i + 1] else ""
                when {
                    pair == "[[" -> { stack.addLast("]]"); i += 2; continue }
                    pair == "((" -> { stack.addLast("))"); i += 2; continue }
                    pair == "]]" -> {
                        if (stack.lastOrNull() == "]]") stack.removeLast()
                        i += 2; continue
                    }
                    pair == "))" -> {
                        if (stack.lastOrNull() == "))") stack.removeLast()
                        i += 2; continue
                    }
                    c == '(' -> stack.addLast(")")
                    c == '[' -> stack.addLast("]")
                    c == '{' -> stack.addLast("}")
                    c == ')' || c == ']' || c == '}' ->
                        if (stack.lastOrNull() == c.toString()) stack.removeLast()
                    c == '"' || c == '\'' -> {
                        val q = c
                        i++
                        while (i < line.length && line[i] != q) {
                            if (line[i] == '\\' && i + 1 < line.length) i++
                            i++
                        }
                    }
                    c == '#' && (i == 0 || line[i - 1].isWhitespace()) -> break
                }
                i++
            }
            if (stack.isEmpty()) return null
            // Don't insert `}` here — function-decl strategy owns the
            // brace block. Also skip if the trailing-comment region
            // would split the close from the line content.
            val closers = stack.reversed().joinToString("")
            if (closers.any { it == '}' }) return null

            // Insert before any trailing line comment.
            val anchor = lineStart + lengthBeforeTrailingComment(line)
            return Plan(anchor, closers, closers.length)
        }

        // ── Strategy 4: bare body opener (`then` / `do` / `in`) ──

        private fun tryBareBodyOpener(
            line: String,
            lineStart: Int,
            text: CharSequence,
        ): Plan? {
            val trimmed = line.trimEnd()
            val lastWord = trimmed.takeLastWhile { !it.isWhitespace() && it != ';' }
            if (lastWord !in BODY_OPENER_WORDS) return null
            // Only fire if the matching closer isn't already on a
            // following line — otherwise we'd just be inserting an
            // empty body line into an already-shaped block.
            val needed = BARE_OPENER_TO_CLOSER[lastWord] ?: return null
            if (followingKeywordPresent(text, lineStart, line, needed)) return null

            val indent = leadingIndent(line)
            val afterLine = lineStart + line.trimEnd().length
            val insert = "\n$indent    \n$indent$needed"
            val caretRel = 1 + indent.length + 4
            return Plan(afterLine, insert, caretRel)
        }

        // ── Shared helpers ───────────────────────────────────────

        /** Find the matching close for [open] at [openIdx]. -1 if unbalanced. */
        private fun findMatchingClose(line: String, openIdx: Int, open: Char, close: Char): Int {
            var depth = 0
            var i = openIdx
            while (i < line.length) {
                val c = line[i]
                when (c) {
                    open -> depth++
                    close -> {
                        depth--
                        if (depth == 0) return i
                    }
                    '"', '\'' -> {
                        val q = c
                        i++
                        while (i < line.length && line[i] != q) {
                            if (line[i] == '\\' && i + 1 < line.length) i++
                            i++
                        }
                    }
                    '#' -> if (i == 0 || line[i - 1].isWhitespace()) return -1
                }
                i++
            }
            return -1
        }

        /** True when [line] contains any unmatched `[[` / `((` / `(` / `[` / `{`. */
        private fun lineHasUnclosedBracket(line: String): Boolean =
            tryBracketBalance(line, 0) != null

        /**
         * True when [line] has an unmatched `{` outside strings /
         * comments — the user already started the body on this line.
         */
        private fun lineHasOpenBrace(line: String): Boolean {
            var depth = 0
            var i = 0
            while (i < line.length) {
                val c = line[i]
                when (c) {
                    '{' -> depth++
                    '}' -> if (depth > 0) depth--
                    '"', '\'' -> {
                        val q = c
                        i++
                        while (i < line.length && line[i] != q) {
                            if (line[i] == '\\' && i + 1 < line.length) i++
                            i++
                        }
                    }
                    '#' -> if (i == 0 || line[i - 1].isWhitespace()) return depth > 0
                }
                i++
            }
            return depth > 0
        }

        /** True when whitespace-then-`{` follows [offset] in [text]. */
        private fun hasOpenBraceAfter(text: CharSequence, offset: Int): Boolean {
            var i = offset
            while (i < text.length) {
                val c = text[i]
                if (c == '{') return true
                if (c != ' ' && c != '\t' && c != '\n' && c != '\r') return false
                i++
            }
            return false
        }

        /** True when [word] appears as a standalone keyword outside strings on [line]. */
        private fun containsKeywordOutsideStrings(line: String, word: String): Boolean {
            var i = 0
            while (i < line.length) {
                val c = line[i]
                when (c) {
                    '"', '\'' -> {
                        val q = c
                        i++
                        while (i < line.length && line[i] != q) {
                            if (line[i] == '\\' && i + 1 < line.length) i++
                            i++
                        }
                    }
                    '#' -> if (i == 0 || line[i - 1].isWhitespace()) return false
                    else -> {
                        if (c.isLetter() || c == '_') {
                            val start = i
                            while (i < line.length && (line[i].isLetterOrDigit() || line[i] == '_')) i++
                            if (line.substring(start, i) == word) {
                                // Word boundary check — surrounding
                                // chars must not be ident chars.
                                val leftOk = start == 0 ||
                                    !line[start - 1].isLetterOrDigit() && line[start - 1] != '_'
                                val rightOk = i == line.length ||
                                    !line[i].isLetterOrDigit() && line[i] != '_'
                                if (leftOk && rightOk) return true
                            }
                            continue
                        }
                    }
                }
                i++
            }
            return false
        }

        /**
         * True when [word] appears as a standalone token on any
         * following line in [text] before EOF — used to suppress
         * closer insertion when the user has already typed `fi` /
         * `done` / `esac` on a later line.
         */
        private fun followingKeywordPresent(
            text: CharSequence,
            lineStart: Int,
            line: String,
            word: String,
        ): Boolean {
            val from = lineStart + line.length
            var i = from
            while (i < text.length) {
                // Scan token at line start (skip leading whitespace).
                while (i < text.length && (text[i] == ' ' || text[i] == '\t')) i++
                if (i + word.length <= text.length &&
                    text.subSequence(i, i + word.length).toString() == word
                ) {
                    val afterEnd = i + word.length
                    val rightOk = afterEnd == text.length ||
                        !text[afterEnd].isLetterOrDigit() && text[afterEnd] != '_'
                    if (rightOk) return true
                }
                // Advance to next line.
                while (i < text.length && text[i] != '\n') i++
                if (i < text.length) i++
            }
            return false
        }

        /** Length of [line] up to the start of any trailing `# ...` comment. */
        private fun lengthBeforeTrailingComment(line: String): Int {
            var i = 0
            while (i < line.length) {
                val c = line[i]
                when (c) {
                    '"', '\'' -> {
                        val q = c
                        i++
                        while (i < line.length && line[i] != q) {
                            if (line[i] == '\\' && i + 1 < line.length) i++
                            i++
                        }
                        if (i < line.length) i++
                        continue
                    }
                    '#' -> if (i == 0 || line[i - 1].isWhitespace()) {
                        // Trailing comment — strip trailing whitespace
                        // before the `#`.
                        var j = i
                        while (j > 0 && (line[j - 1] == ' ' || line[j - 1] == '\t')) j--
                        return j
                    }
                }
                i++
            }
            return line.trimEnd().length
        }

        /** True when [line] (sans trailing whitespace) ends with [word] as a token. */
        private fun lineEndsWith(line: String, word: String): Boolean {
            val trimmed = line.trimEnd()
            if (!trimmed.endsWith(word)) return false
            val pre = trimmed.length - word.length
            return pre == 0 || !trimmed[pre - 1].isLetterOrDigit() && trimmed[pre - 1] != '_'
        }

        /** True if line ends with a bare identifier (for `case NAME` detection). */
        private fun lineEndsWithBareToken(line: String): Boolean {
            val trimmed = line.trimEnd()
            val last = trimmed.lastOrNull() ?: return false
            return last.isLetterOrDigit() || last == '_' || last == '$' || last == '}'
        }

        private fun leadingIndent(line: String): String {
            val end = line.indexOfFirst { it != ' ' && it != '\t' }
            return if (end < 0) line else line.substring(0, end)
        }

        // ── Keyword tables ──────────────────────────────────────

        /** All control keywords this strategy recognises at line start. */
        private val CONTROL_KEYWORD = Regex(
            "(if|elif|else|while|until|for|foreach|select|repeat|case|do|then)\\b",
        )

        /** Body opener for each header keyword (e.g. `if` → `then`). */
        private val BODY_OPENERS = mapOf(
            "if"      to "then",
            "elif"    to "then",
            "while"   to "do",
            "until"   to "do",
            "for"     to "do",
            "foreach" to "do",
            "select"  to "do",
            "repeat"  to "do",
            "case"    to "in",
        )

        /** Body closer for each header keyword (null when none — e.g. `elif`). */
        private val BODY_CLOSERS = mapOf(
            "if"      to "fi",
            "while"   to "done",
            "until"   to "done",
            "for"     to "done",
            "foreach" to "done",
            "select"  to "done",
            "repeat"  to "done",
            "case"    to "esac",
        )

        /** Bare body opener words (Strategy 4). */
        private val BODY_OPENER_WORDS = setOf("then", "do", "in")
        private val BARE_OPENER_TO_CLOSER = mapOf(
            "then" to "fi",
            "do"   to "done",
            "in"   to "esac",
        )

        /** `function NAME` or `function NAME()`. */
        private val FUNCTION_DECL = Regex("function\\s+[A-Za-z_][\\w:.-]*")

        /** POSIX function: `NAME()` standalone on its line. */
        private val POSIX_FUNC_DECL = Regex("([A-Za-z_][\\w:.-]*)\\s*\\(\\s*\\)")
    }
}
