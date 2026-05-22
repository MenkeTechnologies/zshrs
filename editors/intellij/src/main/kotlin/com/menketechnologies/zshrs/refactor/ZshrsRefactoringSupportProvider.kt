package com.menketechnologies.zshrs.refactor

import com.intellij.lang.refactoring.RefactoringSupportProvider
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.SelectionModel
import com.intellij.openapi.project.Project
import com.intellij.platform.lsp.api.LspServer
import com.intellij.platform.lsp.api.LspServerManager
import com.intellij.platform.lsp.api.customization.LspIntentionAction
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.refactoring.RefactoringActionHandler
import com.menketechnologies.zshrs.lsp.ZshrsLspServerSupportProvider
import org.eclipse.lsp4j.CodeAction
import org.eclipse.lsp4j.CodeActionContext
import org.eclipse.lsp4j.CodeActionParams
import org.eclipse.lsp4j.Position
import org.eclipse.lsp4j.Range
import org.eclipse.lsp4j.TextDocumentIdentifier
import org.eclipse.lsp4j.jsonrpc.messages.Either

/**
 * Bridges IntelliJ's keymap-driven refactoring actions (Cmd-Opt-M /
 * Cmd-Opt-V / Cmd-Opt-C / Ctrl-T "Refactor This") into the LSP code
 * actions returned by `zshrs --lsp`.
 */
class ZshrsRefactoringSupportProvider : RefactoringSupportProvider() {
    override fun isAvailable(context: PsiElement): Boolean = true
    override fun isMemberInplaceRenameAvailable(element: PsiElement, context: PsiElement?): Boolean = true
    override fun isInplaceRenameAvailable(element: PsiElement, context: PsiElement?): Boolean = true

    override fun getExtractMethodHandler(): RefactoringActionHandler =
        LspExtractActionHandler { it.contains("function") || it.contains("method") }

    override fun getIntroduceVariableHandler(): RefactoringActionHandler =
        LspExtractActionHandler { it.contains("variable") && !it.contains("constant") }

    override fun getIntroduceConstantHandler(): RefactoringActionHandler =
        LspExtractActionHandler { it.contains("constant") }

    override fun getIntroduceParameterHandler(): RefactoringActionHandler =
        LspExtractActionHandler { it.contains("parameter") }
}

private class LspExtractActionHandler(
    private val titleMatches: (String) -> Boolean,
) : RefactoringActionHandler {

    override fun invoke(
        project: Project,
        editor: Editor?,
        file: PsiFile?,
        dataContext: DataContext?,
    ) {
        if (editor == null || file == null) return
        val virtualFile = file.virtualFile ?: return
        val server = findZshrsLspServer(project) ?: return

        val selection = editor.selectionModel
        // Caret-only invocation: don't block here. The LSP server
        // snaps to the word at the cursor (identifier outside a
        // string, or word-piece inside one). Same UX as stryke's
        // Cmd-Opt-V/C on a bare word.
        val (range, _hasSelection) = selectionRange(editor.document, selection)

        val params = CodeActionParams(
            TextDocumentIdentifier(server.getDocumentIdentifier(virtualFile).uri),
            range,
            CodeActionContext(emptyList()),
        )

        val response: List<Either<org.eclipse.lsp4j.Command, CodeAction>>? = server.sendRequestSync(
            LspServer.DEFAULT_REQUEST_TIMEOUT_MS,
        ) { lsp4j -> lsp4j.textDocumentService.codeAction(params) }

        val match: CodeAction = response
            ?.mapNotNull { e -> if (e.isRight) e.right else null }
            ?.firstOrNull { titleMatches(it.title.lowercase()) }
            ?: return

        val intention = LspIntentionAction(server, match)
        intention.invoke(project, editor, file)
    }

    override fun invoke(
        project: Project,
        elements: Array<PsiElement>,
        dataContext: DataContext?,
    ) {}

    private fun selectionRange(
        document: com.intellij.openapi.editor.Document,
        selection: SelectionModel,
    ): Pair<Range, Boolean> {
        val startOffset = selection.selectionStart
        val endOffset = selection.selectionEnd
        val hasSelection = startOffset != endOffset
        val startLine = document.getLineNumber(startOffset)
        val endLine = document.getLineNumber(endOffset)
        val startCol = startOffset - document.getLineStartOffset(startLine)
        val endCol = endOffset - document.getLineStartOffset(endLine)
        return Range(
            Position(startLine, startCol),
            Position(endLine, endCol),
        ) to hasSelection
    }

    private fun findZshrsLspServer(project: Project): LspServer? =
        LspServerManager.getInstance(project)
            .getServersForProvider(ZshrsLspServerSupportProvider::class.java)
            .firstOrNull()
}
