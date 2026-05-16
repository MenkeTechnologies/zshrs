package com.menketechnologies.zshrs.dap

import com.google.gson.JsonObject
import com.intellij.openapi.application.ApplicationManager
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.evaluation.XDebuggerEvaluator

class ZshrsEvaluator(
    private val client: ZshrsDapClient?,
    private val frameId: Int,
) : XDebuggerEvaluator() {

    override fun evaluate(
        expression: String,
        callback: XEvaluationCallback,
        expressionPosition: XSourcePosition?,
    ) {
        val c = client
        if (c == null || !c.isAlive()) {
            callback.errorOccurred("Debugger not connected")
            return
        }
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val args = JsonObject().apply {
                    addProperty("expression", expression)
                    addProperty("frameId", frameId)
                    addProperty("context", "watch")
                }
                val body = c.request("evaluate", args)
                if (body == null) {
                    callback.errorOccurred("Evaluation timed out")
                    return@executeOnPooledThread
                }
                val result = body.get("result")?.asString ?: ""
                val varRef = body.get("variablesReference")?.asInt ?: 0
                val kind = body.get("type")?.asString ?: "scalar"
                callback.evaluated(ZshrsValue(name = expression, repr = result, kind = kind, varRef = varRef, client = c))
            } catch (e: Exception) {
                callback.errorOccurred(e.message ?: "Evaluation failed")
            }
        }
    }
}
