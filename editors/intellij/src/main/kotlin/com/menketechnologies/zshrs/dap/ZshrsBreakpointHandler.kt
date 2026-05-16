package com.menketechnologies.zshrs.dap

import com.google.gson.JsonArray
import com.google.gson.JsonObject
import com.intellij.openapi.application.ReadAction
import com.intellij.xdebugger.breakpoints.XBreakpointHandler
import com.intellij.xdebugger.breakpoints.XBreakpointProperties
import com.intellij.xdebugger.breakpoints.XLineBreakpoint

class ZshrsBreakpointHandler(
    private val process: ZshrsDebugProcess,
) : XBreakpointHandler<XLineBreakpoint<XBreakpointProperties<*>>>(ZshrsBreakpointType::class.java) {

    @Suppress("UNCHECKED_CAST")
    override fun registerBreakpoint(bp: XLineBreakpoint<XBreakpointProperties<*>>) {
        resync(bp.fileUrl)
    }

    @Suppress("UNCHECKED_CAST")
    override fun unregisterBreakpoint(bp: XLineBreakpoint<XBreakpointProperties<*>>, temporary: Boolean) {
        resync(bp.fileUrl)
    }

    private fun resync(fileUrl: String) {
        val path = fileUrl.removePrefix("file://")
        val client = process.client ?: return
        val bps: List<Int> = ReadAction.compute<List<Int>, RuntimeException> {
            val all = com.intellij.xdebugger.XDebuggerManager.getInstance(process.session.project)
                .breakpointManager
                .getBreakpoints(ZshrsBreakpointType::class.java)
            all.filter { it.fileUrl == fileUrl && it.isEnabled }.map { it.line + 1 }
        }
        val args = JsonObject().apply {
            add("source", JsonObject().apply { addProperty("path", path) })
            val arr = JsonArray()
            for (line in bps) {
                val b = JsonObject()
                b.addProperty("line", line)
                arr.add(b)
            }
            add("breakpoints", arr)
        }
        client.requestAsync("setBreakpoints", args)
    }
}
