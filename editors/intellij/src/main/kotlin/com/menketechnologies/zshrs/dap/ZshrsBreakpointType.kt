package com.menketechnologies.zshrs.dap

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.xdebugger.breakpoints.XLineBreakpointTypeBase
import com.menketechnologies.zshrs.ZshrsSettings

/**
 * Line-breakpoint type for zsh files. The runtime decides at execution time
 * whether a line is reachable; we accept any line of a supported file so the
 * gutter stays uniform.
 */
class ZshrsBreakpointType : XLineBreakpointTypeBase(
    "zshrs-line",
    "zshrs Line Breakpoint",
    ZshrsDebuggerEditorsProvider(),
) {
    override fun canPutAt(file: VirtualFile, line: Int, project: Project): Boolean =
        ZshrsSettings.getInstance().isSupportedFile(file.name, file.extension)

    override fun getPriority(): Int = 100
}
