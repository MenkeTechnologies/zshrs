package com.menketechnologies.zshrs.run

import com.intellij.execution.actions.ConfigurationContext
import com.intellij.execution.actions.LazyRunConfigurationProducer
import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.openapi.util.Ref
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.menketechnologies.zshrs.ZshrsSettings

class ZshrsRunConfigurationProducer : LazyRunConfigurationProducer<ZshrsRunConfiguration>() {

    override fun getConfigurationFactory(): ConfigurationFactory =
        ZshrsRunConfigurationType.getInstance().factory

    override fun setupConfigurationFromContext(
        config: ZshrsRunConfiguration,
        context: ConfigurationContext,
        sourceElement: Ref<PsiElement>,
    ): Boolean {
        val file: PsiFile = context.psiLocation?.containingFile ?: return false
        val vf = file.virtualFile ?: return false
        if (!ZshrsSettings.getInstance().isSupportedFile(vf.name, vf.extension)) return false
        config.options.scriptPath = vf.path
        config.name = vf.nameWithoutExtension.ifBlank { vf.name }
        if (config.options.workingDirectory.isNullOrBlank()) {
            config.options.workingDirectory = vf.parent?.path ?: ""
        }
        if (config.options.noRcs == false && ZshrsSettings.getInstance().defaultNoRcs) {
            config.options.noRcs = true
        }
        return true
    }

    override fun isConfigurationFromContext(
        config: ZshrsRunConfiguration,
        context: ConfigurationContext,
    ): Boolean {
        val vf = context.psiLocation?.containingFile?.virtualFile ?: return false
        return ZshrsSettings.getInstance().isSupportedFile(vf.name, vf.extension) &&
            config.options.scriptPath == vf.path
    }
}
