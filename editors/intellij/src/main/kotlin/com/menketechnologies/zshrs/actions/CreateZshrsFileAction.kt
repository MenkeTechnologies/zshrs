package com.menketechnologies.zshrs.actions

import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.actions.CreateFileFromTemplateDialog
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiFile
import com.intellij.psi.PsiFileFactory
import com.menketechnologies.zshrs.ZshrsFileType
import com.menketechnologies.zshrs.ZshrsIcons

/// File > New > Zsh File. Hands the user a name dialog with a few
/// canonical starting templates (shebanged script, rc fragment,
/// function library, plain). All templates resolve to `ZshrsFileType`
/// so the new buffer immediately picks up syntax highlighting, LSP,
/// commenter, etc. without an IDE reload.
///
/// Implemented via the platform's `CreateFileFromTemplateAction` so
/// we inherit the standard New-File dialog (name field, template
/// picker, undoable PSI write). Templates are inline string literals
/// here rather than `fileTemplates/internal/*.zsh` so the plugin
/// stays single-jar with no resource extraction at runtime.
class CreateZshrsFileAction :
    CreateFileFromTemplateAction("Zsh File", "Create new zsh script", ZshrsIcons.FILE) {

    override fun getActionName(directory: PsiDirectory?, newName: String, templateName: String?): String =
        "Create Zsh File"

    override fun buildDialog(
        project: Project,
        directory: PsiDirectory,
        builder: CreateFileFromTemplateDialog.Builder,
    ) {
        builder
            .setTitle("New Zsh File")
            .addKind("Script (#!/usr/bin/env zshrs)", ZshrsIcons.FILE, TPL_SCRIPT)
            .addKind("Function library",                 ZshrsIcons.FILE, TPL_LIB)
            .addKind("Rc fragment",                      ZshrsIcons.FILE, TPL_RC)
            .addKind("Empty",                            ZshrsIcons.FILE, TPL_EMPTY)
    }

    override fun createFile(name: String, templateName: String, dir: PsiDirectory): PsiFile? {
        val fileName = if (name.contains('.')) name else "$name.zsh"
        val body = when (templateName) {
            TPL_SCRIPT -> SCRIPT_BODY
            TPL_LIB    -> LIB_BODY
            TPL_RC     -> RC_BODY
            else       -> ""
        }
        val file = PsiFileFactory.getInstance(dir.project)
            .createFileFromText(fileName, ZshrsFileType, body)
        return dir.add(file) as? PsiFile
    }

    companion object {
        private const val TPL_SCRIPT = "Script"
        private const val TPL_LIB    = "Library"
        private const val TPL_RC     = "Rc"
        private const val TPL_EMPTY  = "Empty"

        private val SCRIPT_BODY = """
            |#!/usr/bin/env zshrs
            |# vim:ft=zsh
            |
            |set -euo pipefail
            |
            |main() {
            |    print -- "hello from zshrs"
            |}
            |
            |main "${'$'}@"
            |""".trimMargin()

        private val LIB_BODY = """
            |# Function library — source this from another script.
            |#
            |# Usage:
            |#   source ${'$'}{0:A:h}/this-file.zsh
            |
            |my-fn() {
            |    print -- "called my-fn with ${'$'}*"
            |}
            |""".trimMargin()

        private val RC_BODY = """
            |# .zshrc fragment — source from your main rc file.
            |#
            |# Keep one concern per file. Use the autoload pattern for
            |# anything large enough that startup cost matters.
            |
            |setopt extended_glob no_case_glob hist_ignore_dups
            |
            |alias ll='ls -lah'
            |""".trimMargin()
    }
}
