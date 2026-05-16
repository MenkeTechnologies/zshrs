package com.menketechnologies.zshrs.run

import com.intellij.openapi.fileChooser.FileChooserDescriptorFactory
import com.intellij.openapi.options.SettingsEditor
import com.intellij.openapi.ui.TextFieldWithBrowseButton
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import com.intellij.util.ui.JBUI
import javax.swing.DefaultComboBoxModel
import javax.swing.JComboBox
import javax.swing.JComponent
import javax.swing.JPanel

class ZshrsRunConfigurationEditor : SettingsEditor<ZshrsRunConfiguration>() {
    private val scriptField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "zsh Script",
            "Choose a zsh script to run",
            null,
            FileChooserDescriptorFactory.createSingleFileNoJarsDescriptor(),
        )
    }
    private val scriptArgsField = JBTextField()
    private val interpreterArgsField = JBTextField()
    private val workDirField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "Working Directory",
            "Choose the run working directory",
            null,
            FileChooserDescriptorFactory.createSingleFolderDescriptor(),
        )
    }
    private val noRcsCheck = JBCheckBox("-f / --no-rcs (skip startup files)")
    private val xtraceCheck = JBCheckBox("-x (xtrace)")
    private val verboseCheck = JBCheckBox("-v (verbose)")
    private val disasmCheck = JBCheckBox("--disasm (fusevm bytecode disassembly)")
    private val dumpAstCheck = JBCheckBox("--dump-ast (parser AST → stderr)")
    private val compatCombo: JComboBox<String> = JComboBox(
        DefaultComboBoxModel(arrayOf("default", "zsh", "bash", "ksh", "posix"))
    )

    private val panel: JPanel = FormBuilder.createFormBuilder()
        .addComponent(header("Program"))
        .addLabeledComponent("Script:", scriptField)
        .addLabeledComponent("Script arguments:", scriptArgsField)
        .addLabeledComponent("Interpreter arguments:", interpreterArgsField)
        .addLabeledComponent("Working directory:", workDirField)

        .addComponent(header("Compat mode"))
        .addLabeledComponent("Mode:", compatCombo)
        .addTooltip("default = zshrs. `zsh` / `bash` / `ksh` / `posix` are identical-behaviour drop-ins.")

        .addComponent(header("Flags"))
        .addComponent(noRcsCheck)
        .addComponent(xtraceCheck)
        .addComponent(verboseCheck)

        .addComponent(header("Tracing / debug"))
        .addComponent(disasmCheck)
        .addComponent(dumpAstCheck)

        .addComponentFillVertically(JPanel(), 0)
        .panel.apply { border = JBUI.Borders.empty(8) }

    private fun header(title: String) =
        JBLabel("<html><b>$title</b></html>").apply { border = JBUI.Borders.emptyTop(8) }

    override fun createEditor(): JComponent = panel

    override fun resetEditorFrom(s: ZshrsRunConfiguration) {
        scriptField.text = s.options.scriptPath.orEmpty()
        scriptArgsField.text = s.options.scriptArgs.orEmpty()
        interpreterArgsField.text = s.options.interpreterArgs.orEmpty()
        workDirField.text = s.options.workingDirectory.orEmpty()
        noRcsCheck.isSelected = s.options.noRcs
        xtraceCheck.isSelected = s.options.xtrace
        verboseCheck.isSelected = s.options.verbose
        disasmCheck.isSelected = s.options.disasm
        dumpAstCheck.isSelected = s.options.dumpAst
        compatCombo.selectedItem = s.options.compatMode ?: "default"
    }

    override fun applyEditorTo(s: ZshrsRunConfiguration) {
        s.options.scriptPath = scriptField.text
        s.options.scriptArgs = scriptArgsField.text
        s.options.interpreterArgs = interpreterArgsField.text
        s.options.workingDirectory = workDirField.text
        s.options.noRcs = noRcsCheck.isSelected
        s.options.xtrace = xtraceCheck.isSelected
        s.options.verbose = verboseCheck.isSelected
        s.options.disasm = disasmCheck.isSelected
        s.options.dumpAst = dumpAstCheck.isSelected
        s.options.compatMode = compatCombo.selectedItem as? String
    }
}
