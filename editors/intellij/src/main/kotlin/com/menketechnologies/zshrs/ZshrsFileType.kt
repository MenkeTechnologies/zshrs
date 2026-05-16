package com.menketechnologies.zshrs

import com.intellij.openapi.fileTypes.LanguageFileType
import javax.swing.Icon

object ZshrsFileType : LanguageFileType(ZshrsLanguage) {
    override fun getName(): String = "zsh"
    override fun getDescription(): String = "zsh / zshrs shell script"
    override fun getDefaultExtension(): String = "zsh"
    override fun getIcon(): Icon = ZshrsIcons.FILE
}
