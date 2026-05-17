package com.menketechnologies.zshrs.refactor

import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.vfs.VirtualFileManager
import com.intellij.platform.lsp.api.LspServer
import com.intellij.platform.lsp.api.LspServerManager
import com.intellij.platform.lsp.util.applyTextEdits
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.refactoring.rename.RenameHandler
import com.menketechnologies.zshrs.ZshrsFileType
import com.menketechnologies.zshrs.lsp.ZshrsLspServerSupportProvider
import org.eclipse.lsp4j.Position
import org.eclipse.lsp4j.RenameParams
import org.eclipse.lsp4j.TextDocumentIdentifier
import org.eclipse.lsp4j.WorkspaceEdit

/**
 * Handles Shift-F6 (Rename) and Ctrl-T → Rename on zsh files via the
 * LSP server's `textDocument/rename` endpoint.
 */
class ZshrsRenameHandler : RenameHandler {

    override fun isAvailableOnDataContext(dataContext: DataContext): Boolean {
        val file = dataContext.getData(CommonDataKeys.PSI_FILE) ?: return false
        return file.fileType == ZshrsFileType
    }

    override fun isRenaming(dataContext: DataContext): Boolean = isAvailableOnDataContext(dataContext)

    override fun invoke(project: Project, editor: Editor?, file: PsiFile?, dataContext: DataContext?) {
        if (editor == null || file == null) return
        val virtualFile = file.virtualFile ?: return
        val server = LspServerManager.getInstance(project)
            .getServersForProvider(ZshrsLspServerSupportProvider::class.java)
            .firstOrNull() ?: return

        val offset = editor.caretModel.offset
        val doc = editor.document
        val line = doc.getLineNumber(offset)
        val col = offset - doc.getLineStartOffset(line)
        val pos = Position(line, col)

        val identifier = identifierAt(doc.charsSequence, offset)
        val newName = Messages.showInputDialog(
            project,
            "Rename '${identifier.ifEmpty { "<identifier>" }}' to:",
            "Rename",
            null,
            identifier,
            null,
        ) ?: return
        if (newName.isBlank() || newName == identifier) return

        val params = RenameParams(
            TextDocumentIdentifier(server.getDocumentIdentifier(virtualFile).uri),
            pos,
            newName,
        )
        val edit: WorkspaceEdit? = server.sendRequestSync(
            LspServer.DEFAULT_REQUEST_TIMEOUT_MS,
        ) { lsp4j -> lsp4j.textDocumentService.rename(params) }

        if (edit == null) return

        val changes = edit.changes ?: emptyMap()
        WriteCommandAction.runWriteCommandAction(project) {
            for ((uri, edits) in changes) {
                val vf = VirtualFileManager.getInstance().findFileByUrl(uri) ?: continue
                val document = FileDocumentManager.getInstance().getDocument(vf) ?: continue
                applyTextEdits(document, edits)
            }
        }
        FileDocumentManager.getInstance().saveAllDocuments()
    }

    override fun invoke(project: Project, elements: Array<PsiElement>, dataContext: DataContext?) {}

    private fun identifierAt(chars: CharSequence, offset: Int): String {
        if (offset < 0 || offset > chars.length) return ""
        var s = offset
        var e = offset
        while (s > 0 && isIdentChar(chars[s - 1])) s--
        while (e < chars.length && isIdentChar(chars[e])) e++
        if (s == e) return ""
        return chars.subSequence(s, e).toString()
    }

    private fun isIdentChar(c: Char): Boolean = c == '_' || c.isLetterOrDigit()
}
