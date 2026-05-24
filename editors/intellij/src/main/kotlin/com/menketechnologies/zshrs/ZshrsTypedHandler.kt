package com.menketechnologies.zshrs

import com.intellij.codeInsight.editorActions.TypedHandlerDelegate
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileTypes.FileType
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiFile

/**
 * Skip-over typed-handler for `}` and `)` and `]`.
 *
 * Without this, typing `{` in zsh source auto-pairs to `{}` (good),
 * but typing the closing brace AGAIN — either reflexively, or after
 * smart-enter expanded the pair across two lines — inserts a SECOND
 * `}` next to the existing one instead of advancing the cursor past
 * it. The user reported: `tomm(){<ENTER>}` produced `tomm(){\n}\n}`
 * — the smart-enter put a closing `}` on its own indented line, and
 * the user's reflexive `}` keypress added a duplicate.
 *
 * Skip-over check: if the char to the right of the cursor is already
 * the same close-brace the user typed, swallow the keypress and
 * just move the cursor past the existing brace. Same behavior every
 * IDE-shipped language provides via its custom TypedHandler.
 *
 * Strykelang doesn't have this (yet); zshrs adds it because shell
 * scripts use `{ … }` and `( … )` heavily — duplicate-close was the
 * single most-reported lexer/editor papercut.
 */
class ZshrsTypedHandler : TypedHandlerDelegate() {
    override fun beforeCharTyped(
        c: Char,
        project: Project,
        editor: Editor,
        file: PsiFile,
        fileType: FileType,
    ): Result {
        if (file.fileType !is ZshrsFileType) return Result.CONTINUE
        if (c != '}' && c != ')' && c != ']') return Result.CONTINUE

        val offset = editor.caretModel.offset
        val text = editor.document.charsSequence
        if (offset >= text.length) return Result.CONTINUE
        val nextChar = text[offset]
        if (nextChar != c) return Result.CONTINUE

        // Heuristic: only skip-over when the line up to the cursor has
        // BALANCED counts of the corresponding open / close. If the
        // user actually needs to close a still-open `{`, we shouldn't
        // eat their keypress. Counting on the whole document would be
        // expensive on large files; line-local is good enough for the
        // typical case.
        val lineStart = run {
            var i = offset - 1
            while (i >= 0 && text[i] != '\n') i--
            i + 1
        }
        var depth = 0
        val open = when (c) { '}' -> '{'; ')' -> '('; ']' -> '['; else -> return Result.CONTINUE }
        for (i in lineStart until offset) {
            when (text[i]) {
                open -> depth++
                c -> depth--
            }
        }
        // Already balanced (or close-heavy) — the existing `c` to the
        // right is the matching close; skip over it.
        if (depth <= 0) {
            editor.caretModel.moveToOffset(offset + 1)
            return Result.STOP
        }
        return Result.CONTINUE
    }
}
