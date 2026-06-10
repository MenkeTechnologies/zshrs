package com.menketechnologies.zshrs.toolwindow

import com.google.gson.JsonElement
import com.google.gson.JsonParser
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.process.CapturingProcessHandler
import com.intellij.icons.AllIcons
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.DefaultActionGroup
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.SimpleToolWindowPanel
import com.intellij.openapi.util.SystemInfo
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.SearchTextField
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTabbedPane
import com.intellij.ui.treeStructure.Tree
import com.intellij.util.ui.JBUI
import com.menketechnologies.zshrs.ZshrsSettings
import java.awt.BorderLayout
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.io.File
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.SwingUtilities
import javax.swing.tree.DefaultMutableTreeNode
import javax.swing.tree.DefaultTreeModel

class ZshrsReflectionToolWindowFactory : ToolWindowFactory {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val panel = ZshrsReflectionPanel(project)
        val content = toolWindow.contentManager.factory.createContent(panel, "", false)
        toolWindow.contentManager.addContent(content)
    }

    override fun shouldBeAvailable(project: Project): Boolean = true
}

private class ZshrsReflectionPanel(private val project: Project) : SimpleToolWindowPanel(true, true) {

    private val tabs = JBTabbedPane()
    private val statusLabel = JBLabel(" ")
    private val tabsByName = LinkedHashMap<String, HashTabPanel>()

    init {
        layout = BorderLayout()
        val group = DefaultActionGroup().apply {
            add(RefreshAction())
            addSeparator()
            add(OpenSettingsAction())
        }
        val actionToolbar = com.intellij.openapi.actionSystem.ActionManager.getInstance()
            .createActionToolbar("ZshrsReflectionToolbar", group, true)
        actionToolbar.targetComponent = this
        toolbar = actionToolbar.component

        val body = JPanel(BorderLayout())
        body.add(tabs, BorderLayout.CENTER)
        body.add(statusLabel.apply { border = JBUI.Borders.empty(4, 8) }, BorderLayout.SOUTH)
        setContent(body)

        reload()
    }

    private fun reload() {
        statusLabel.text = "Loading reflection data from `zshrs --dump-reflection`…"
        tabs.removeAll()
        tabsByName.clear()
        ApplicationManager.getApplication().executeOnPooledThread {
            val (data, err) = loadReflection()
            SwingUtilities.invokeLater {
                if (data == null) {
                    statusLabel.text = "Failed to load: ${err.orEmpty()}"
                    return@invokeLater
                }
                for ((name, json) in data.entrySet()) {
                    val tab = HashTabPanel(project, name, json)
                    tabsByName[name] = tab
                    tabs.addTab(tabTitle(name, tab.entryCount), tab)
                }
                statusLabel.text = "${data.entrySet().sumOf { it.value.asJsonObject.size() }} entries across ${data.size()} categories"
            }
        }
    }

    private fun tabTitle(name: String, count: Int): String = "${prettyName(name)}  $count"

    private fun loadReflection(): Pair<com.google.gson.JsonObject?, String?> {
        val exe = resolveExe() ?: return null to "zshrs executable not found (Settings → Tools → zshrs)"
        return try {
            val cmd = GeneralCommandLine(exe, "--dump-reflection").withCharset(Charsets.UTF_8)
            val handler = CapturingProcessHandler(cmd)
            val out = handler.runProcess(15_000)
            if (out.exitCode != 0) return null to "exit ${out.exitCode}: ${out.stderr.take(400)}"
            val json = JsonParser.parseString(out.stdout).asJsonObject
            json to null
        } catch (e: Exception) {
            LOG.warn("reflection dump failed", e)
            null to (e.message ?: e.javaClass.simpleName)
        }
    }

    private fun resolveExe(): String? {
        val cfg = ZshrsSettings.getInstance().zshrsExecutable
        if (!cfg.isNullOrBlank() && File(cfg).canExecute()) return cfg
        val pathEnv = System.getenv("PATH") ?: return null
        val suffixes = if (SystemInfo.isWindows) listOf(".exe", ".bat", ".cmd", "") else listOf("")
        for (dir in pathEnv.split(File.pathSeparator)) {
            for (suf in suffixes) {
                val f = File(dir, "zshrs" + suf)
                if (f.canExecute()) return f.absolutePath
            }
        }
        return null
    }

    private fun prettyName(name: String): String = when (name) {
        "all"          -> "All"
        "builtins"     -> "Builtins"
        "compat"       -> "Compat"
        "keywords"     -> "Keywords"
        "options"      -> "Options"
        "parameters"   -> "Parameters"
        "special_vars" -> "Special vars"
        "param_flags"  -> "Param flags"
        "redirects"    -> "Redirects"
        "aliases"      -> "Aliases"
        "compsys"      -> "Compsys"
        "extensions"   -> "Extensions"
        "operators"    -> "Operators"
        else -> name.replaceFirstChar { it.titlecase() }
    }

    private inner class RefreshAction : AnAction("Refresh", "Re-run `zshrs --dump-reflection` and `zshrs --dump-plugins`, reload tabs and External Libraries", AllIcons.Actions.Refresh) {
        override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.BGT
        override fun actionPerformed(e: AnActionEvent) {
            reload()
            // Also re-fetch the plugin list so External Libraries picks
            // up newly-sourced zsh plugins without an IDE restart.
            com.menketechnologies.zshrs.library.ZshrsPluginRegistry
                .getInstance(project)
                .refreshAsync()
        }
    }

    private inner class OpenSettingsAction : AnAction("Settings", "Open zshrs settings", AllIcons.General.Settings) {
        override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.BGT
        override fun actionPerformed(e: AnActionEvent) {
            com.intellij.openapi.options.ShowSettingsUtil.getInstance()
                .showSettingsDialog(project, "zshrs")
        }
    }

    companion object {
        private val LOG = Logger.getInstance(ZshrsReflectionPanel::class.java)
    }
}

private class HashTabPanel(
    private val project: Project,
    private val hashName: String,
    private val data: JsonElement,
) : JPanel(BorderLayout()) {

    private val root = DefaultMutableTreeNode(hashName)
    private val model = DefaultTreeModel(root)
    private val tree = Tree(model).apply {
        isRootVisible = false
        showsRootHandles = true
    }
    private val search = SearchTextField()

    val entryCount: Int

    init {
        val obj = data.asJsonObject
        entryCount = obj.size()

        val byCategory: MutableMap<String, MutableList<String>> = sortedMapOf()
        for ((k, v) in obj.entrySet()) {
            val cat = if (v.isJsonPrimitive) v.asString else v.toString()
            byCategory.getOrPut(cat) { mutableListOf() }.add(k)
        }
        for ((cat, entries) in byCategory) {
            val catNode = DefaultMutableTreeNode("$cat  (${entries.size})")
            for (name in entries.sorted()) {
                catNode.add(DefaultMutableTreeNode(name))
            }
            root.add(catNode)
        }
        model.reload()

        add(buildHeader(), BorderLayout.NORTH)
        add(JBScrollPane(tree), BorderLayout.CENTER)

        tree.addMouseListener(object : MouseAdapter() {
            override fun mouseClicked(e: MouseEvent) {
                if (e.clickCount != 1 || !SwingUtilities.isLeftMouseButton(e) || e.isPopupTrigger) return
                val path = tree.getPathForLocation(e.x, e.y) ?: return
                val node = path.lastPathComponent as? DefaultMutableTreeNode ?: return
                if (!node.isLeaf) return
                val name = (node.userObject as? String) ?: return
                showDocsPopupAt(name, e)
            }
        })

        com.intellij.ui.PopupHandler.installPopupHandler(
            tree,
            buildContextActionGroup(),
            "ZshrsReflectionPopup",
        )

        search.addDocumentListener(object : javax.swing.event.DocumentListener {
            override fun insertUpdate(e: javax.swing.event.DocumentEvent) = applyFilter()
            override fun removeUpdate(e: javax.swing.event.DocumentEvent) = applyFilter()
            override fun changedUpdate(e: javax.swing.event.DocumentEvent) = applyFilter()
        })
    }

    private fun buildHeader(): JComponent {
        val p = JPanel(BorderLayout())
        p.border = JBUI.Borders.empty(4)
        p.add(search, BorderLayout.CENTER)
        return p
    }

    private fun applyFilter() {
        val q = search.text.trim().lowercase()
        root.removeAllChildren()
        val obj = data.asJsonObject
        val byCategory: MutableMap<String, MutableList<String>> = sortedMapOf()
        for ((k, v) in obj.entrySet()) {
            val cat = if (v.isJsonPrimitive) v.asString else v.toString()
            val matches = q.isEmpty() || k.lowercase().contains(q) || cat.lowercase().contains(q)
            if (matches) byCategory.getOrPut(cat) { mutableListOf() }.add(k)
        }
        for ((cat, entries) in byCategory) {
            val catNode = DefaultMutableTreeNode("$cat  (${entries.size})")
            for (name in entries.sorted()) catNode.add(DefaultMutableTreeNode(name))
            root.add(catNode)
        }
        model.reload()
        if (q.isNotEmpty()) {
            var i = 0
            while (i < tree.rowCount) { tree.expandRow(i); i++ }
        }
    }

    private fun showDocsPopupAt(name: String, e: MouseEvent) {
        ApplicationManager.getApplication().executeOnPooledThread {
            val rawText = fetchDocsRaw(name)
            ApplicationManager.getApplication().invokeLater {
                val console = com.intellij.execution.filters.TextConsoleBuilderFactory
                    .getInstance()
                    .createBuilder(project)
                    .console as com.intellij.execution.ui.ConsoleView
                val decoder = com.intellij.execution.process.AnsiEscapeDecoder()
                decoder.escapeText(
                    rawText,
                    com.intellij.execution.process.ProcessOutputTypes.STDOUT,
                ) { fragment, outputType ->
                    val ct = com.intellij.execution.ui.ConsoleViewContentType.getConsoleViewType(outputType)
                    console.print(fragment, ct)
                }
                val component = console.component
                component.preferredSize = java.awt.Dimension(720, 420)
                val popup = com.intellij.openapi.ui.popup.JBPopupFactory.getInstance()
                    .createComponentPopupBuilder(component, console.preferredFocusableComponent)
                    .setTitle("zshrs docs: $name")
                    .setResizable(true)
                    .setMovable(true)
                    .setRequestFocus(true)
                    .setCancelOnClickOutside(true)
                    .createPopup()
                com.intellij.openapi.util.Disposer.register(popup) { console.dispose() }
                popup.show(com.intellij.ui.awt.RelativePoint(e))
            }
        }
    }

    private fun selectedLeafName(): String? {
        val sel = tree.selectionPath ?: return null
        val node = sel.lastPathComponent as? DefaultMutableTreeNode ?: return null
        if (!node.isLeaf) return null
        return node.userObject as? String
    }

    private fun buildContextActionGroup(): com.intellij.openapi.actionSystem.ActionGroup {
        val group = com.intellij.openapi.actionSystem.DefaultActionGroup()
        group.add(ShowDocsAction())
        group.add(CopyNameAction())
        return group
    }

    private inner class ShowDocsAction : com.intellij.openapi.actionSystem.AnAction(
        "Show Docs", "Open the hover doc card for this name", com.intellij.icons.AllIcons.Actions.Help,
    ) {
        override fun getActionUpdateThread() = com.intellij.openapi.actionSystem.ActionUpdateThread.BGT
        override fun update(e: com.intellij.openapi.actionSystem.AnActionEvent) {
            e.presentation.isEnabled = selectedLeafName() != null
        }
        override fun actionPerformed(e: com.intellij.openapi.actionSystem.AnActionEvent) {
            val name = selectedLeafName() ?: return
            showDocsPopup(name, e)
        }
    }

    private inner class CopyNameAction : com.intellij.openapi.actionSystem.AnAction(
        "Copy Name", "Copy this name to the clipboard", com.intellij.icons.AllIcons.Actions.Copy,
    ) {
        override fun getActionUpdateThread() = com.intellij.openapi.actionSystem.ActionUpdateThread.BGT
        override fun update(e: com.intellij.openapi.actionSystem.AnActionEvent) {
            e.presentation.isEnabled = selectedLeafName() != null
        }
        override fun actionPerformed(e: com.intellij.openapi.actionSystem.AnActionEvent) {
            val name = selectedLeafName() ?: return
            val sel = java.awt.datatransfer.StringSelection(name)
            java.awt.Toolkit.getDefaultToolkit().systemClipboard.setContents(sel, sel)
        }
    }

    private fun showDocsPopup(name: String, e: com.intellij.openapi.actionSystem.AnActionEvent) {
        ApplicationManager.getApplication().executeOnPooledThread {
            val rawText = fetchDocsRaw(name)
            ApplicationManager.getApplication().invokeLater {
                val console = com.intellij.execution.filters.TextConsoleBuilderFactory
                    .getInstance()
                    .createBuilder(project)
                    .console as com.intellij.execution.ui.ConsoleView

                val decoder = com.intellij.execution.process.AnsiEscapeDecoder()
                decoder.escapeText(
                    rawText,
                    com.intellij.execution.process.ProcessOutputTypes.STDOUT,
                ) { fragment, outputType ->
                    val ct = com.intellij.execution.ui.ConsoleViewContentType.getConsoleViewType(outputType)
                    console.print(fragment, ct)
                }

                val component: javax.swing.JComponent = console.component
                component.preferredSize = java.awt.Dimension(720, 420)

                val popup = com.intellij.openapi.ui.popup.JBPopupFactory.getInstance()
                    .createComponentPopupBuilder(component, console.preferredFocusableComponent)
                    .setTitle("zshrs docs: $name")
                    .setResizable(true)
                    .setMovable(true)
                    .setRequestFocus(true)
                    .setCancelOnClickOutside(true)
                    .setCancelOnOtherWindowOpen(true)
                    .createPopup()
                com.intellij.openapi.util.Disposer.register(popup) { console.dispose() }
                if (e.inputEvent != null) popup.showInBestPositionFor(e.dataContext)
                else popup.showCenteredInCurrentWindow(project)
            }
        }
    }

    private fun fetchDocsRaw(name: String): String {
        val exe = resolveExe() ?: return "zshrs executable not found — set it in Settings → Tools → zshrs"
        return try {
            // Two attempts: built-in docs subcommand, then `man` fallback
            val cmd = GeneralCommandLine(exe, "--docs", name)
                .withCharset(Charsets.UTF_8)
                .withEnvironment("FORCE_COLOR", "1")
                .withEnvironment("CLICOLOR_FORCE", "1")
            val handler = CapturingProcessHandler(cmd)
            val out = handler.runProcess(8_000)
            val raw = if (out.exitCode == 0) out.stdout else out.stderr.ifBlank { out.stdout }
            raw.trim().ifBlank { "(no docs for $name)" }
        } catch (e: Exception) {
            "Failed to fetch docs: ${e.message}"
        }
    }

    private fun resolveExe(): String? {
        val cfg = ZshrsSettings.getInstance().zshrsExecutable
        if (!cfg.isNullOrBlank() && File(cfg).canExecute()) return cfg
        val pathEnv = System.getenv("PATH") ?: return null
        val suffixes = if (SystemInfo.isWindows) listOf(".exe", ".bat", ".cmd", "") else listOf("")
        for (dir in pathEnv.split(File.pathSeparator)) {
            for (suf in suffixes) {
                val f = File(dir, "zshrs" + suf)
                if (f.canExecute()) return f.absolutePath
            }
        }
        return null
    }
}
