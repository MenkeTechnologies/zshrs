package com.menketechnologies.zshrs.dap

import com.google.gson.JsonObject
import com.intellij.icons.AllIcons
import com.intellij.openapi.application.ApplicationManager
import com.intellij.xdebugger.frame.XCompositeNode
import com.intellij.xdebugger.frame.XNamedValue
import com.intellij.xdebugger.frame.XValueChildrenList
import com.intellij.xdebugger.frame.XValueNode
import com.intellij.xdebugger.frame.XValuePlace

/**
 * One variable rendered in the Variables tool window.
 *
 * Scalars (varRef == 0) are leaves. Arrays / hashes (varRef != 0) advertise
 * a non-zero `variablesReference` so the IDE shows an expand triangle and
 * calls [computeChildren] on click.
 */
class ZshrsValue(
    name: String,
    private val repr: String,
    private val kind: String,
    private val varRef: Int = 0,
    private val client: ZshrsDapClient? = null,
) : XNamedValue(name) {

    override fun computePresentation(node: XValueNode, place: XValuePlace) {
        val icon = when (kind) {
            "array" -> AllIcons.Debugger.Db_array
            "hash", "assoc" -> AllIcons.Debugger.Db_dep_field_breakpoint
            else -> AllIcons.Debugger.Value
        }
        node.setPresentation(icon, kind, repr, varRef != 0)
    }

    override fun computeChildren(node: XCompositeNode) {
        if (varRef == 0 || client == null) {
            node.addChildren(XValueChildrenList.EMPTY, true)
            return
        }
        ApplicationManager.getApplication().executeOnPooledThread {
            val args = JsonObject().apply { addProperty("variablesReference", varRef) }
            val body = client.request("variables", args)
            val list = XValueChildrenList()
            val arr = body?.getAsJsonArray("variables")
            if (arr != null) {
                for (v in arr) {
                    val vo = v.asJsonObject
                    list.add(
                        ZshrsValue(
                            name = vo.get("name")?.asString ?: "?",
                            repr = vo.get("value")?.asString ?: "",
                            kind = vo.get("type")?.asString ?: "",
                            varRef = vo.get("variablesReference")?.asInt ?: 0,
                            client = client,
                        )
                    )
                }
            }
            node.addChildren(list, true)
        }
    }

    override fun canNavigateToSource(): Boolean = false
}
