package com.menketechnologies.zshrs.run

import com.intellij.execution.configurations.RunProfile
import com.intellij.execution.executors.DefaultRunExecutor
import com.intellij.execution.runners.DefaultProgramRunner

class ZshrsProgramRunner : DefaultProgramRunner() {
    override fun getRunnerId(): String = "ZshrsProgramRunner"

    override fun canRun(executorId: String, profile: RunProfile): Boolean {
        if (profile !is ZshrsRunConfiguration) return false
        return executorId == DefaultRunExecutor.EXECUTOR_ID
    }
}
