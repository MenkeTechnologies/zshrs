package com.menketechnologies.zshrs.lsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.application.PathManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import com.intellij.platform.lsp.api.customization.LspCodeActionsSupport
import com.intellij.platform.lsp.api.customization.LspCompletionSupport
import com.intellij.platform.lsp.api.customization.LspDiagnosticsSupport
import com.intellij.platform.lsp.api.customization.LspFormattingSupport
import com.intellij.platform.lsp.api.customization.LspSemanticTokensSupport
import com.menketechnologies.zshrs.ZshrsColors
import com.menketechnologies.zshrs.ZshrsSettings
import java.io.File

class ZshrsLspServerDescriptor(project: Project) :
    ProjectWideLspServerDescriptor(project, "zshrs") {

    override fun isSupportedFile(file: VirtualFile): Boolean =
        ZshrsSettings.getInstance().isSupportedFile(file.name, file.extension)

    // ── Explicit feature opt-ins (2024.2 deprecated-API style) ────────────
    // The default `LspSemanticTokensSupport()` returns null from
    // `getTextAttributesKey` — so even if the server sends semantic
    // tokens, the IDE has no color slot to apply and the overlay is
    // silently dropped. Map every LSP token type our server emits.

    override val lspSemanticTokensSupport: LspSemanticTokensSupport = object : LspSemanticTokensSupport() {
        override fun getTextAttributesKey(
            tokenType: String,
            tokenModifiers: List<String>,
        ): com.intellij.openapi.editor.colors.TextAttributesKey? = when (tokenType) {
            "keyword" -> ZshrsColors.KEYWORD
            "function" -> ZshrsColors.BUILTIN
            "variable" -> ZshrsColors.VARIABLE
            "parameter" -> ZshrsColors.VARIABLE
            "string" -> ZshrsColors.STRING_DQ
            "number" -> ZshrsColors.NUMBER
            "comment" -> ZshrsColors.COMMENT
            "operator" -> ZshrsColors.OPERATOR
            "macro" -> ZshrsColors.FUNCTION_DECL
            "type" -> ZshrsColors.OPTION_NAME
            "class" -> ZshrsColors.OPTION_NAME
            "property" -> ZshrsColors.VARIABLE
            "namespace" -> ZshrsColors.OPTION_NAME
            // zshrs-defined LSP semantic types — see SEMANTIC_TOKEN_TYPES
            // in src/extensions/lsp.rs. Drive distinct color slots so
            // ext + compsys names don't visually merge with compat builtins.
            "zshrsExtension" -> ZshrsColors.EXTENSION_BUILTIN
            "zshrsCompsys" -> ZshrsColors.COMPSYS_FUNCTION
            else -> null
        }
    }

    override val lspCodeActionsSupport: LspCodeActionsSupport = LspCodeActionsSupport()
    override val lspDiagnosticsSupport: LspDiagnosticsSupport = LspDiagnosticsSupport()
    /// Re-trigger completion popup after inserting an item whose
    /// LSP `command` is `editor.action.triggerSuggest`. zsh lets you
    /// chain glob qualifiers (`*(/D^.)`), param flags (`${(LU)var}`),
    /// pattern modifiers (`(#ib)`), subscript flags (`${arr[(Ri)x]}`),
    /// and `:` modifiers (`${var:h:t:r}`) — without this hook the
    /// popup closes after the first insertion and the user has to
    /// press Ctrl-Space again to pick another. The Platform LSP API's
    /// default `LspCompletionSupport` doesn't honor the `command`
    /// field on completion items; this subclass adds that behavior.
    override val lspCompletionSupport: LspCompletionSupport = object : LspCompletionSupport() {
        override fun createLookupElement(
            parameters: com.intellij.codeInsight.completion.CompletionParameters,
            item: org.eclipse.lsp4j.CompletionItem,
        ): com.intellij.codeInsight.lookup.LookupElement? {
            val base = super.createLookupElement(parameters, item) ?: return null
            val cmd = item.command ?: return base
            if (cmd.command != "editor.action.triggerSuggest") return base
            val editor = parameters.editor
            val proj = editor.project ?: project
            return com.intellij.codeInsight.lookup.LookupElementDecorator
                .withDelegateInsertHandler<com.intellij.codeInsight.lookup.LookupElement>(
                    base,
                ) { ctx, _ ->
                    // The decorator already wraps `base`; invoke its
                    // insert handler via the captured reference. The
                    // lambda's second arg is the decorator itself
                    // (whose `delegate` property is private in the
                    // current Platform API), so we use `base` instead.
                    base.handleInsert(ctx)
                    // Schedule auto-popup at the new cursor position
                    // after the IDE finishes the insertion + caret-move.
                    ctx.setLaterRunnable {
                        com.intellij.codeInsight.AutoPopupController
                            .getInstance(proj)
                            .scheduleAutoPopup(editor)
                    }
                }
        }
    }
    override val lspFormattingSupport: LspFormattingSupport = LspFormattingSupport()
    override val lspHoverSupport: Boolean = true
    override val lspGoToDefinitionSupport: Boolean = true

    override fun createCommandLine(): GeneralCommandLine {
        val settings = ZshrsSettings.getInstance()
        val exe = resolveExe()
        LOG.info("Starting zshrs LSP: $exe --lsp ${settings.extraLspArgs}")
        com.menketechnologies.zshrs.ZshrsDebugLog.log(
            "lsp",
            "createCommandLine exe=$exe args=--lsp ${settings.extraLspArgs} cwd=${project.basePath}",
        )
        val cmd = GeneralCommandLine(exe)
            .withParameters("--lsp")
            .withWorkDirectory(project.basePath ?: PathManager.getHomePath())
            .withEnvironment("RUST_BACKTRACE", "1")
        splitArgs(settings.extraLspArgs).forEach { cmd.addParameter(it) }
        for (kv in splitArgs(settings.lspEnv)) {
            val i = kv.indexOf('=')
            if (i > 0) cmd.withEnvironment(kv.substring(0, i), kv.substring(i + 1))
        }
        if (settings.logLspToFile && settings.lspLogPath.isNotBlank()) {
            cmd.withEnvironment("ZSHRS_LSP_LOG", settings.lspLogPath)
            com.menketechnologies.zshrs.ZshrsDebugLog.log(
                "lsp",
                "ZSHRS_LSP_LOG=${settings.lspLogPath}",
            )
        }
        return cmd
    }

    private fun resolveExe(): String {
        val settings = ZshrsSettings.getInstance()
        settings.zshrsExecutable
            ?.takeIf { it.isNotBlank() && File(it).canExecute() }
            ?.let { return it }
        return findOnPath("zshrs") ?: "zshrs"
    }

    private fun findOnPath(name: String): String? {
        val pathEnv = System.getenv("PATH") ?: return null
        val sep = File.pathSeparator
        val suffixes = if (SystemInfo.isWindows) listOf(".exe", ".bat", ".cmd", "") else listOf("")
        for (dir in pathEnv.split(sep)) {
            for (suf in suffixes) {
                val f = File(dir, name + suf)
                if (f.canExecute()) return f.absolutePath
            }
        }
        return null
    }

    private fun splitArgs(s: String): List<String> {
        if (s.isBlank()) return emptyList()
        val out = mutableListOf<String>()
        val sb = StringBuilder()
        var quote: Char? = null
        for (c in s) {
            when {
                quote != null && c == quote -> quote = null
                quote != null -> sb.append(c)
                c == '"' || c == '\'' -> quote = c
                c.isWhitespace() -> if (sb.isNotEmpty()) { out += sb.toString(); sb.clear() }
                else -> sb.append(c)
            }
        }
        if (sb.isNotEmpty()) out += sb.toString()
        return out
    }

    companion object {
        private val LOG = Logger.getInstance(ZshrsLspServerDescriptor::class.java)
    }
}
