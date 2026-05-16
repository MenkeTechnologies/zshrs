package com.menketechnologies.zshrs.dap

import com.intellij.xdebugger.frame.XExecutionStack
import com.intellij.xdebugger.frame.XStackFrame
import com.intellij.xdebugger.frame.XSuspendContext

class ZshrsSuspendContext(private val stack: ZshrsExecutionStack) : XSuspendContext() {
    override fun getActiveExecutionStack(): XExecutionStack = stack
}

class ZshrsExecutionStack : XExecutionStack("Main") {

    @Volatile private var frames: List<ZshrsStackFrame> = emptyList()

    fun setFrames(newFrames: List<ZshrsStackFrame>) {
        frames = newFrames
    }

    override fun getTopFrame(): XStackFrame? = frames.firstOrNull()

    override fun computeStackFrames(firstFrameIndex: Int, container: XStackFrameContainer) {
        val slice = if (firstFrameIndex <= 0) frames else frames.drop(firstFrameIndex)
        container.addStackFrames(slice, true)
    }
}
