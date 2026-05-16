package com.menketechnologies.zshrs

import com.intellij.lang.Language

object ZshrsLanguage : Language("zshrs") {
    private fun readResolve(): Any = ZshrsLanguage
    override fun getDisplayName(): String = "zshrs"
    override fun isCaseSensitive(): Boolean = true
}
