package com.menketechnologies.zshrs

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.util.xmlb.XmlSerializerUtil

@Service(Service.Level.APP)
@State(name = "ZshrsSettings", storages = [Storage("zshrs.xml")])
class ZshrsSettings : PersistentStateComponent<ZshrsSettings.State> {
    data class State(
        var zshrsExecutable: String? = null,
        var lspEnabled: Boolean = true,
        var extraLspArgs: String = "",
        var defaultNoRcs: Boolean = false,
        var disableLexerHighlighting: Boolean = false,
        var fileExtensions: String = "zsh",
        var autoRestartLsp: Boolean = true,
        var lspEnv: String = "",
        var logLspToFile: Boolean = false,
        var lspLogPath: String = "",
        var enableBuiltinHovers: Boolean = true,
    )

    private var stateData = State()

    override fun getState(): State = stateData
    override fun loadState(state: State) { XmlSerializerUtil.copyBean(state, stateData) }

    var zshrsExecutable: String?
        get() = stateData.zshrsExecutable
        set(value) { stateData.zshrsExecutable = value }
    var lspEnabled: Boolean
        get() = stateData.lspEnabled
        set(value) { stateData.lspEnabled = value }
    var extraLspArgs: String
        get() = stateData.extraLspArgs
        set(value) { stateData.extraLspArgs = value }
    var defaultNoRcs: Boolean
        get() = stateData.defaultNoRcs
        set(value) { stateData.defaultNoRcs = value }
    var disableLexerHighlighting: Boolean
        get() = stateData.disableLexerHighlighting
        set(value) { stateData.disableLexerHighlighting = value }
    var fileExtensions: String
        get() = stateData.fileExtensions
        set(value) { stateData.fileExtensions = value }
    var autoRestartLsp: Boolean
        get() = stateData.autoRestartLsp
        set(value) { stateData.autoRestartLsp = value }
    var lspEnv: String
        get() = stateData.lspEnv
        set(value) { stateData.lspEnv = value }
    var logLspToFile: Boolean
        get() = stateData.logLspToFile
        set(value) { stateData.logLspToFile = value }
    var lspLogPath: String
        get() = stateData.lspLogPath
        set(value) { stateData.lspLogPath = value }
    var enableBuiltinHovers: Boolean
        get() = stateData.enableBuiltinHovers
        set(value) { stateData.enableBuiltinHovers = value }

    fun supportedExtensions(): List<String> =
        fileExtensions.split(",", " ", ";")
            .map { it.trim().removePrefix(".") }
            .filter { it.isNotEmpty() }

    /** Match a virtual file's name/extension against the configured set. */
    fun isSupportedFile(filename: String, extension: String?): Boolean {
        if (extension != null && extension in supportedExtensions()) return true
        // Recognized dotfile bases regardless of extension
        return filename in DOTFILES
    }

    companion object {
        private val DOTFILES = setOf(
            ".zshrc", ".zshenv", ".zlogin", ".zlogout", ".zprofile", ".zpreztorc",
            "zshrc", "zshenv", "zlogin", "zlogout", "zprofile",
        )

        fun getInstance(): ZshrsSettings =
            ApplicationManager.getApplication().getService(ZshrsSettings::class.java)
    }
}
