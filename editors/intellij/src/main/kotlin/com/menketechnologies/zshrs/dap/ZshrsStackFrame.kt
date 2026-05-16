package com.menketechnologies.zshrs.dap

import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.ui.ColoredTextContainer
import com.intellij.ui.SimpleTextAttributes
import com.intellij.xdebugger.XDebuggerUtil
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.evaluation.XDebuggerEvaluator
import com.intellij.xdebugger.frame.XCompositeNode
import com.intellij.xdebugger.frame.XStackFrame
import com.intellij.xdebugger.frame.XValueChildrenList

class ZshrsStackFrame(
    private val client: ZshrsDapClient?,
    private val frameId: Int,
    private val name: String,
    private val file: String,
    private val line: Int,
    private val children: List<ZshrsValue>,
) : XStackFrame() {

    override fun getSourcePosition(): XSourcePosition? {
        if (file.isBlank()) return null
        val vf = LocalFileSystem.getInstance().refreshAndFindFileByPath(file) ?: return null
        return XDebuggerUtil.getInstance().createPosition(vf, (line - 1).coerceAtLeast(0))
    }

    override fun computeChildren(node: XCompositeNode) {
        val list = XValueChildrenList()
        for (c in children) list.add(c)
        node.addChildren(list, true)
    }

    override fun getEvaluator(): XDebuggerEvaluator = ZshrsEvaluator(client, frameId)

    override fun customizePresentation(component: ColoredTextContainer) {
        val label = if (name.isBlank()) "frame@${frameId} (${shortFile()}:$line)"
                    else "$name (${shortFile()}:$line)"
        component.append(label, SimpleTextAttributes.REGULAR_ATTRIBUTES)
    }

    private fun shortFile(): String = file.substringAfterLast('/').ifBlank { "<unknown>" }
}
