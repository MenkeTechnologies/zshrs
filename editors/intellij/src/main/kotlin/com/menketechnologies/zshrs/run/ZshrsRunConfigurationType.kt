package com.menketechnologies.zshrs.run

import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.execution.configurations.ConfigurationType
import com.intellij.execution.configurations.RunConfiguration
import com.intellij.openapi.project.Project
import com.menketechnologies.zshrs.ZshrsIcons
import javax.swing.Icon

class ZshrsRunConfigurationType : ConfigurationType {
    override fun getDisplayName(): String = "zshrs"
    override fun getConfigurationTypeDescription(): String = "Run a zsh script with zshrs"
    override fun getIcon(): Icon = ZshrsIcons.FILE
    override fun getId(): String = "ZSHRS_RUN_CONFIGURATION"
    override fun getConfigurationFactories(): Array<ConfigurationFactory> = arrayOf(factory)

    val factory = object : ConfigurationFactory(this) {
        override fun getId(): String = "zshrs"
        override fun createTemplateConfiguration(project: Project): RunConfiguration =
            ZshrsRunConfiguration(project, this, "zshrs")
        override fun getOptionsClass(): Class<ZshrsRunConfigurationOptions> =
            ZshrsRunConfigurationOptions::class.java
    }

    companion object {
        fun getInstance(): ZshrsRunConfigurationType =
            com.intellij.execution.configurations.ConfigurationTypeUtil
                .findConfigurationType(ZshrsRunConfigurationType::class.java)
    }
}
