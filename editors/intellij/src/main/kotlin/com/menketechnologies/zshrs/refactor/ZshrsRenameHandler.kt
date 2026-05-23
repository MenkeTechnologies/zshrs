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
        dbg("invoked")
        if (editor == null) { dbg("ABORT: no editor"); return }
        if (file == null) { dbg("ABORT: no file"); return }
        val virtualFile = file.virtualFile ?: run { dbg("ABORT: no virtualFile"); return }
        val server = LspServerManager.getInstance(project)
            .getServersForProvider(ZshrsLspServerSupportProvider::class.java)
            .firstOrNull()
        if (server == null) { dbg("ABORT: no LSP server"); return }

        val offset = editor.caretModel.offset
        val doc = editor.document
        val line = doc.getLineNumber(offset)
        val col = offset - doc.getLineStartOffset(line)
        val pos = Position(line, col)

        val identifier = identifierAt(doc.charsSequence, offset)
        dbg("caret line=$line col=$col identifier='$identifier'")

        val newName = Messages.showInputDialog(
            project,
            "Rename '${identifier.ifEmpty { "<identifier>" }}' to:",
            "Rename",
            null,
            identifier,
            null,
        )
        if (newName == null) { dbg("ABORT: user cancelled"); return }
        if (newName.isBlank()) { dbg("ABORT: blank newName"); return }
        if (newName == identifier) { dbg("ABORT: unchanged"); return }
        dbg("newName='$newName'")

        val params = RenameParams(
            TextDocumentIdentifier(server.getDocumentIdentifier(virtualFile).uri),
            pos,
            newName,
        )
        dbg("sending textDocument/rename uri=${params.textDocument.uri}")
        val edit: WorkspaceEdit? = try {
            server.sendRequestSync(LspServer.DEFAULT_REQUEST_TIMEOUT_MS) { lsp4j ->
                lsp4j.textDocumentService.rename(params)
            }
        } catch (t: Throwable) {
            dbg("EXCEPTION sending rename: ${t::class.java.simpleName}: ${t.message}")
            Messages.showErrorDialog(project, "LSP rename request failed: ${t.message}", "Rename")
            return
        }
        if (edit == null) {
            dbg("ABORT: LSP returned null WorkspaceEdit")
            return
        }
        dbg("got WorkspaceEdit changes=${edit.changes?.size ?: 0} documentChanges=${edit.documentChanges?.size ?: 0}")

        WriteCommandAction.runWriteCommandAction(project) {
            var totalEdits = 0
            edit.changes?.forEach { (uri, edits) ->
                val vf = VirtualFileManager.getInstance().findFileByUrl(uri)
                if (vf == null) { dbg("no VirtualFile for uri=$uri"); return@forEach }
                val document = FileDocumentManager.getInstance().getDocument(vf)
                if (document == null) { dbg("no Document for $uri"); return@forEach }
                dbg("applying ${edits.size} edits to $uri")
                applyTextEdits(document, edits)
                totalEdits += edits.size
            }
            edit.documentChanges?.forEach { dc ->
                if (dc.isLeft) {
                    val tde = dc.left ?: return@forEach
                    val uri = tde.textDocument.uri
                    val vf = VirtualFileManager.getInstance().findFileByUrl(uri)
                    if (vf == null) { dbg("no VirtualFile for uri=$uri (docChanges)"); return@forEach }
                    val document = FileDocumentManager.getInstance().getDocument(vf)
                    if (document == null) { dbg("no Document for $uri (docChanges)"); return@forEach }
                    dbg("applying ${tde.edits.size} edits to $uri (docChanges)")
                    applyTextEdits(document, tde.edits)
                    totalEdits += tde.edits.size
                } else {
                    dbg("skipping non-edit documentChange: ${dc.right?.javaClass?.simpleName}")
                }
            }
            dbg("totalEdits applied = $totalEdits")
        }
        FileDocumentManager.getInstance().saveAllDocuments()
        dbg("done")
    }

    private fun dbg(msg: String) {
        com.menketechnologies.zshrs.ZshrsDebugLog.log("rename", msg)
    }

    override fun invoke(project: Project, elements: Array<PsiElement>, dataContext: DataContext?) {}

    /**
     * Bare identifier span at `offset` — DOES NOT walk through `::`
     * package / module separators. Cursor on `handle` inside
     * `Demo::handle` returns just `"handle"`, not the qualified form.
     *
     * Why this matters: if `identifierAt` consumed `::` to produce
     * qualified prefills like `"Demo::handle"`, the user would edit
     * the suffix in the dialog (e.g. type `handle2`), the dialog
     * returns the WHOLE prefilled string with the new suffix
     * (`"Demo::handle2"`), and the LSP server uses that whole string
     * as the bare replacement — splicing the qualifier in at every
     * match site and producing nonsense like `Demo::Demo::handle2`.
     * The LSP server resolves the target symbol from the cursor
     * POSITION, not the dialog prefill, so the bare segment is always
     * sufficient. The LSP also defensively strips any trailing
     * `::`-qualifier from `newName` as a second line of defense, but
     * the right fix is here at the source.
     *
     * zsh function names don't natively use `::`, but compsys and
     * perl-style user codebases (`Module::sub_name`) do — same trap.
     */
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
