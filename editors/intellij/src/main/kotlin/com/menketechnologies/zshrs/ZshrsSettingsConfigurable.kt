package com.menketechnologies.zshrs

import com.intellij.openapi.fileChooser.FileChooserDescriptorFactory
import com.intellij.openapi.options.Configurable
import com.intellij.openapi.ui.TextFieldWithBrowseButton
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import com.intellij.util.ui.JBUI
import javax.swing.JComponent
import javax.swing.JPanel

class ZshrsSettingsConfigurable : Configurable {

    private val executableField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "zshrs Executable",
            "Path to the zshrs binary",
            null,
            FileChooserDescriptorFactory.createSingleFileNoJarsDescriptor(),
        )
    }
    private val lspEnabledBox = JBCheckBox("Enable LSP (uses `zshrs --lsp`)")
    private val extraLspArgsField = JBTextField()
    private val defaultNoRcsBox = JBCheckBox("Default new run configs to `--no-rcs` (-f)")
    private val disableLexerBox = JBCheckBox("Disable lexer highlighting (rely on LSP semantic tokens only)")
    private val fileExtensionsField = JBTextField()
    private val claimShFilesBox = JBCheckBox("Also claim `.sh` files (treat as zshrs)")
    private val autoRestartBox = JBCheckBox("Auto-restart LSP after settings change")
    private val lspEnvField = JBTextField()
    private val enableHoversBox = JBCheckBox("Show server-provided builtin hovers")
    private val logToFileBox = JBCheckBox("Log LSP traffic to file")
    private val lspLogPathField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "LSP Log Path",
            "Where to write the LSP traffic log",
            null,
            FileChooserDescriptorFactory.createSingleFileNoJarsDescriptor(),
        )
    }

    private var panel: JPanel? = null

    override fun getDisplayName(): String = "zshrs"

    override fun createComponent(): JComponent {
        val p = FormBuilder.createFormBuilder()
            .addComponent(sectionHeader("Interpreter"))
            .addLabeledComponent(JBLabel("zshrs executable:"), executableField, 1, false)
            .addTooltip("Leave blank to use the first `zshrs` on \$PATH.")

            .addComponent(sectionHeader("LSP"))
            .addComponent(lspEnabledBox)
            .addLabeledComponent(JBLabel("Extra LSP args:"), extraLspArgsField, 1, false)
            .addTooltip("Whitespace-separated. Passed after `--lsp` when starting the server.")
            .addLabeledComponent(JBLabel("LSP environment:"), lspEnvField, 1, false)
            .addTooltip("`KEY=VAL` pairs, whitespace-separated. e.g. RUST_LOG=info")
            .addComponent(autoRestartBox)
            .addComponent(enableHoversBox)
            .addComponent(logToFileBox)
            .addLabeledComponent(JBLabel("LSP log file:"), lspLogPathField, 1, false)

            .addComponent(sectionHeader("Editor"))
            .addComponent(disableLexerBox)
            .addComponent(claimShFilesBox)
            .addTooltip("Off by default — turn on if you save zsh scripts with `.sh` so portable shells can run them.")
            .addLabeledComponent(JBLabel("File extensions:"), fileExtensionsField, 1, false)
            .addTooltip("Comma-separated, no leading dot. Default: `zsh`. Use this for extensions beyond `.sh` (e.g. `bash, ksh`).")

            .addComponent(sectionHeader("Run configurations"))
            .addComponent(defaultNoRcsBox)

            .addComponentFillVertically(JPanel(), 0)
            .panel
        p.border = JBUI.Borders.empty(10)
        panel = p
        reset()
        return p
    }

    private fun sectionHeader(title: String) =
        JBLabel("<html><b>$title</b></html>").apply { border = JBUI.Borders.emptyTop(8) }

    override fun isModified(): Boolean {
        val s = ZshrsSettings.getInstance()
        return executableField.text != (s.zshrsExecutable ?: "") ||
            lspEnabledBox.isSelected != s.lspEnabled ||
            extraLspArgsField.text != s.extraLspArgs ||
            defaultNoRcsBox.isSelected != s.defaultNoRcs ||
            disableLexerBox.isSelected != s.disableLexerHighlighting ||
            fileExtensionsField.text != s.fileExtensions ||
            autoRestartBox.isSelected != s.autoRestartLsp ||
            lspEnvField.text != s.lspEnv ||
            enableHoversBox.isSelected != s.enableBuiltinHovers ||
            logToFileBox.isSelected != s.logLspToFile ||
            lspLogPathField.text != s.lspLogPath ||
            claimShFilesBox.isSelected != s.claimShFiles
    }

    override fun apply() {
        val s = ZshrsSettings.getInstance()
        s.zshrsExecutable = executableField.text.takeIf { it.isNotBlank() }
        s.lspEnabled = lspEnabledBox.isSelected
        s.extraLspArgs = extraLspArgsField.text
        s.defaultNoRcs = defaultNoRcsBox.isSelected
        s.disableLexerHighlighting = disableLexerBox.isSelected
        s.fileExtensions = fileExtensionsField.text.ifBlank { "zsh" }
        s.autoRestartLsp = autoRestartBox.isSelected
        s.lspEnv = lspEnvField.text
        s.enableBuiltinHovers = enableHoversBox.isSelected
        s.logLspToFile = logToFileBox.isSelected
        s.lspLogPath = lspLogPathField.text
        s.claimShFiles = claimShFilesBox.isSelected
        // Re-apply dynamic file-extension associations so the change
        // takes effect immediately — without this the user would have
        // to restart the IDE for `.sh` files to start using zshrs.
        ZshrsExtensionAssociator.applyDynamicAssociations()
    }

    override fun reset() {
        val s = ZshrsSettings.getInstance()
        executableField.text = s.zshrsExecutable ?: ""
        lspEnabledBox.isSelected = s.lspEnabled
        extraLspArgsField.text = s.extraLspArgs
        defaultNoRcsBox.isSelected = s.defaultNoRcs
        disableLexerBox.isSelected = s.disableLexerHighlighting
        fileExtensionsField.text = s.fileExtensions
        autoRestartBox.isSelected = s.autoRestartLsp
        lspEnvField.text = s.lspEnv
        enableHoversBox.isSelected = s.enableBuiltinHovers
        logToFileBox.isSelected = s.logLspToFile
        lspLogPathField.text = s.lspLogPath
        claimShFilesBox.isSelected = s.claimShFiles
    }

    override fun disposeUIResources() { panel = null }
}
