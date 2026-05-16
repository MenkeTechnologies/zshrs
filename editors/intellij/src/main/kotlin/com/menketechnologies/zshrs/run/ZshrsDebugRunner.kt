package com.menketechnologies.zshrs.run

import com.intellij.execution.ExecutionException
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.configurations.RunProfile
import com.intellij.execution.configurations.RunProfileState
import com.intellij.execution.executors.DefaultDebugExecutor
import com.intellij.execution.process.KillableColoredProcessHandler
import com.intellij.execution.runners.DefaultProgramRunner
import com.intellij.execution.runners.ExecutionEnvironment
import com.intellij.execution.ui.RunContentDescriptor
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.util.io.FileUtil
import com.intellij.xdebugger.XDebugProcess
import com.intellij.xdebugger.XDebugProcessStarter
import com.intellij.xdebugger.XDebugSession
import com.intellij.xdebugger.XDebuggerManager
import com.menketechnologies.zshrs.ZshrsSettings
import com.menketechnologies.zshrs.dap.ZshrsDebugProcess

/**
 * Debug executor for [ZshrsRunConfiguration]. Spawns `zshrs --dap 127.0.0.1:N`,
 * accepts the DAP socket connection, then constructs an [XDebugProcess] that
 * speaks DAP over that socket while program stdio flows through the process
 * handler into the Debug Console.
 */
class ZshrsDebugRunner : DefaultProgramRunner() {
    override fun getRunnerId(): String = "ZshrsDebugRunner"

    override fun canRun(executorId: String, profile: RunProfile): Boolean =
        executorId == DefaultDebugExecutor.EXECUTOR_ID && profile is ZshrsRunConfiguration

    @Throws(ExecutionException::class)
    override fun doExecute(state: RunProfileState, env: ExecutionEnvironment): RunContentDescriptor? {
        val cfg = env.runProfile as ZshrsRunConfiguration
        val exe = ZshrsSettings.getInstance().zshrsExecutable
            ?.takeIf { it.isNotBlank() } ?: "zshrs"

        val serverSocket = java.net.ServerSocket()
        serverSocket.reuseAddress = true
        serverSocket.bind(java.net.InetSocketAddress("127.0.0.1", 0))
        val port = serverSocket.localPort

        val cmd = GeneralCommandLine()
            .withExePath(exe)
            .withCharset(Charsets.UTF_8)
            .withParameters("--dap", "127.0.0.1:$port")
        val wd = cfg.options.workingDirectory?.takeIf { it.isNotBlank() }
            ?: FileUtil.toSystemDependentName(env.project.basePath ?: ".")
        cmd.withWorkDirectory(wd)
        val handler = KillableColoredProcessHandler(cmd)

        serverSocket.soTimeout = 10_000
        val dapSocket: java.net.Socket = try {
            serverSocket.accept()
        } catch (e: java.net.SocketTimeoutException) {
            handler.destroyProcess()
            throw ExecutionException(
                "zshrs debug: `zshrs --dap 127.0.0.1:$port` didn't connect within 10s. " +
                    "Is the binary fresh? (Settings → Tools → zshrs executable)",
            )
        }
        serverSocket.close()

        val session: XDebugSession = XDebuggerManager.getInstance(env.project).startSession(
            env,
            object : XDebugProcessStarter() {
                override fun start(session: XDebugSession): XDebugProcess {
                    val args = splitArgs(cfg.options.scriptArgs.orEmpty())
                    return ZshrsDebugProcess(
                        session = session,
                        processHandler = handler,
                        dapSocket = dapSocket,
                        programPath = cfg.options.scriptPath.orEmpty(),
                        programArgs = args,
                        workingDirectory = wd,
                    )
                }
            },
        )

        return getDescriptorWithoutSplitDebuggerWarning(session)
            ?: @Suppress("DEPRECATION") session.runContentDescriptor
    }

    private fun getDescriptorWithoutSplitDebuggerWarning(session: XDebugSession): RunContentDescriptor? {
        return try {
            val m = session.javaClass.methods.firstOrNull {
                it.name == "getMockRunContentDescriptorIfInitialized" && it.parameterCount == 0
            } ?: return null
            m.isAccessible = true
            m.invoke(session) as? RunContentDescriptor
        } catch (e: Throwable) {
            LOG.debug("getMockRunContentDescriptorIfInitialized reflection failed", e)
            null
        }
    }

    private fun splitArgs(s: String): List<String> {
        if (s.isBlank()) return emptyList()
        val out = mutableListOf<String>()
        val sb = StringBuilder()
        var quote: Char? = null
        for (c in s) {
            when {
                quote != null && c == quote -> quote = null
                quote != null -> sb.append(c)
                c == '"' || c == '\'' -> quote = c
                c.isWhitespace() -> if (sb.isNotEmpty()) { out += sb.toString(); sb.clear() }
                else -> sb.append(c)
            }
        }
        if (sb.isNotEmpty()) out += sb.toString()
        return out
    }

    companion object {
        private val LOG = Logger.getInstance(ZshrsDebugRunner::class.java)
    }
}
