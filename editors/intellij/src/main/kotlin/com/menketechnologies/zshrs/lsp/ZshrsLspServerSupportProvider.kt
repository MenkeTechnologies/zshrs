package com.menketechnologies.zshrs.lsp

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter
import com.menketechnologies.zshrs.ZshrsSettings

class ZshrsLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerStarter) {
        val settings = ZshrsSettings.getInstance()
        if (!settings.lspEnabled) return
        if (!settings.isSupportedFile(file.name, file.extension)) return
        serverStarter.ensureServerStarted(ZshrsLspServerDescriptor(project))
    }
}
