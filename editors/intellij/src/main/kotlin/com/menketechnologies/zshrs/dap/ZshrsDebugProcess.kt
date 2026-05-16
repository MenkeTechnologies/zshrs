package com.menketechnologies.zshrs.dap

import com.google.gson.JsonArray
import com.google.gson.JsonObject
import com.intellij.execution.process.ProcessHandler
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.xdebugger.XDebugProcess
import com.intellij.xdebugger.XDebugSession
import com.intellij.xdebugger.breakpoints.XBreakpointHandler
import com.intellij.xdebugger.evaluation.XDebuggerEditorsProvider

class ZshrsDebugProcess(
    session: XDebugSession,
    private val processHandler: ProcessHandler,
    private val dapSocket: java.net.Socket,
    private val programPath: String,
    private val programArgs: List<String>,
    private val workingDirectory: String?,
) : XDebugProcess(session) {

    @Volatile var client: ZshrsDapClient? = null
        private set

    private val executionStack = ZshrsExecutionStack()
    private val editorsProvider = ZshrsDebuggerEditorsProvider()
    private val breakpointHandlers = arrayOf<XBreakpointHandler<*>>(ZshrsBreakpointHandler(this))

    override fun getEditorsProvider(): XDebuggerEditorsProvider = editorsProvider
    override fun getBreakpointHandlers(): Array<XBreakpointHandler<*>> = breakpointHandlers
    override fun doGetProcessHandler(): ProcessHandler = processHandler

    override fun createConsole(): com.intellij.execution.ui.ExecutionConsole {
        val console = com.intellij.execution.filters.TextConsoleBuilderFactory
            .getInstance()
            .createBuilder(session.project)
            .console as com.intellij.execution.ui.ConsoleView
        console.attachToProcess(processHandler)
        return console
    }

    override fun sessionInitialized() {
        super.sessionInitialized()
        ApplicationManager.getApplication().invokeLater {
            if (!processHandler.isStartNotified) {
                processHandler.startNotify()
            }
        }

        val out = dapSocket.getOutputStream()
        val inp = dapSocket.getInputStream()

        val c = ZshrsDapClient(
            output = out,
            input = inp,
            onEvent = { ev, body -> handleEvent(ev, body) },
            onLog = { /* trace if needed */ },
        )
        client = c

        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                c.request(
                    "initialize",
                    JsonObject().apply {
                        addProperty("clientID", "intellij-zshrs")
                        addProperty("clientName", "IntelliJ zshrs")
                        addProperty("adapterID", "zshrs")
                        addProperty("locale", "en-US")
                        addProperty("linesStartAt1", true)
                        addProperty("columnsStartAt1", true)
                        addProperty("pathFormat", "path")
                        addProperty("supportsVariableType", true)
                        addProperty("supportsRunInTerminalRequest", false)
                        addProperty("supportsProgressReporting", false)
                    },
                )
                sendAllBreakpoints()
                c.request("configurationDone")
                val launchArgs = JsonObject().apply {
                    addProperty("program", programPath)
                    addProperty("stopOnEntry", false)
                    val args = JsonArray()
                    programArgs.forEach { args.add(it) }
                    add("args", args)
                    workingDirectory?.let { addProperty("cwd", it) }
                }
                c.request("launch", launchArgs)
            } catch (t: Throwable) {
                LOG.warn("DAP init sequence failed", t)
            }
        }
    }

    private fun sendAllBreakpoints() {
        val byFile = mutableMapOf<String, MutableList<Int>>()
        val mgr = com.intellij.xdebugger.XDebuggerManager.getInstance(session.project).breakpointManager
        for (bp in mgr.getBreakpoints(ZshrsBreakpointType::class.java)) {
            if (!bp.isEnabled) continue
            val path = bp.fileUrl.removePrefix("file://")
            byFile.getOrPut(path) { mutableListOf() }.add(bp.line + 1)
        }
        val c = client ?: return
        for ((path, lines) in byFile) {
            val args = JsonObject().apply {
                add("source", JsonObject().apply { addProperty("path", path) })
                val arr = JsonArray()
                for (l in lines) {
                    arr.add(JsonObject().apply { addProperty("line", l) })
                }
                add("breakpoints", arr)
            }
            c.requestAsync("setBreakpoints", args)
        }
    }

    private fun handleEvent(event: String, body: JsonObject) {
        when (event) {
            "stopped" -> onStopped(body)
            "terminated" -> session.stop()
            "exited" -> session.stop()
            "output" -> {
                val text = body.get("output")?.asString ?: return
                val category = body.get("category")?.asString ?: "stdout"
                val outputType = when (category) {
                    "stderr" -> com.intellij.execution.process.ProcessOutputTypes.STDERR
                    "console" -> com.intellij.execution.process.ProcessOutputTypes.SYSTEM
                    else -> com.intellij.execution.process.ProcessOutputTypes.STDOUT
                }
                processHandler.notifyTextAvailable(text, outputType)
            }
            else -> { /* informational */ }
        }
    }

    private fun onStopped(body: JsonObject) {
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val c = client ?: return@executeOnPooledThread

                val stArgs = JsonObject().apply {
                    addProperty("threadId", 1)
                    addProperty("startFrame", 0)
                    addProperty("levels", 100)
                }
                val stBody = c.request("stackTrace", stArgs) ?: return@executeOnPooledThread
                val rawFrames = stBody.getAsJsonArray("stackFrames") ?: return@executeOnPooledThread
                if (rawFrames.size() == 0) return@executeOnPooledThread

                val builtFrames = mutableListOf<ZshrsStackFrame>()
                for (rf in rawFrames) {
                    val fo = rf.asJsonObject
                    val frameId = fo.get("id")?.asInt ?: 0
                    val frameName = fo.get("name")?.asString ?: "<frame>"
                    val frameFile = fo.getAsJsonObject("source")?.get("path")?.asString ?: ""
                    val frameLine = fo.get("line")?.asInt ?: 0

                    val scopesArgs = JsonObject().apply { addProperty("frameId", frameId) }
                    val scopesBody = c.request("scopes", scopesArgs)
                    val scopes = scopesBody?.getAsJsonArray("scopes")

                    val children = mutableListOf<ZshrsValue>()
                    if (scopes != null) {
                        for (s in scopes) {
                            val so = s.asJsonObject
                            val varRef = so.get("variablesReference")?.asInt ?: continue
                            if (varRef == 0) continue
                            val varsArgs = JsonObject().apply { addProperty("variablesReference", varRef) }
                            val varsBody = c.request("variables", varsArgs) ?: continue
                            val vars = varsBody.getAsJsonArray("variables") ?: continue
                            for (v in vars) {
                                val vo = v.asJsonObject
                                children += ZshrsValue(
                                    name = vo.get("name")?.asString ?: "?",
                                    repr = vo.get("value")?.asString ?: "",
                                    kind = vo.get("type")?.asString ?: "scalar",
                                    varRef = vo.get("variablesReference")?.asInt ?: 0,
                                    client = c,
                                )
                            }
                        }
                    }
                    builtFrames += ZshrsStackFrame(
                        client = c,
                        frameId = frameId,
                        name = frameName,
                        file = frameFile,
                        line = frameLine,
                        children = children,
                    )
                }

                executionStack.setFrames(builtFrames)
                val ctx = ZshrsSuspendContext(executionStack)
                ApplicationManager.getApplication().invokeLater {
                    session.positionReached(ctx)
                }
            } catch (t: Throwable) {
                LOG.warn("onStopped fetch failed", t)
            }
        }
    }

    override fun resume(context: com.intellij.xdebugger.frame.XSuspendContext?) {
        client?.requestAsync("continue", JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun startStepOver(context: com.intellij.xdebugger.frame.XSuspendContext?) {
        client?.requestAsync("next", JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun startStepInto(context: com.intellij.xdebugger.frame.XSuspendContext?) {
        client?.requestAsync("stepIn", JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun startStepOut(context: com.intellij.xdebugger.frame.XSuspendContext?) {
        client?.requestAsync("stepOut", JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun startPausing() {
        client?.requestAsync("pause", JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun stop() {
        client?.requestAsync("disconnect", JsonObject().apply { addProperty("terminateDebuggee", true) })
        client?.close()
        try { dapSocket.close() } catch (_: Exception) {}
    }

    override fun runToPosition(position: com.intellij.xdebugger.XSourcePosition, context: com.intellij.xdebugger.frame.XSuspendContext?) {
        val c = client ?: return
        val path = position.file.path
        val line = position.line + 1
        val args = JsonObject().apply {
            add("source", JsonObject().apply { addProperty("path", path) })
            val arr = JsonArray()
            arr.add(JsonObject().apply { addProperty("line", line) })
            add("breakpoints", arr)
        }
        c.requestAsync("setBreakpoints", args)
        c.requestAsync("continue", JsonObject().apply { addProperty("threadId", 1) })
    }

    companion object {
        private val LOG = Logger.getInstance(ZshrsDebugProcess::class.java)
    }
}
