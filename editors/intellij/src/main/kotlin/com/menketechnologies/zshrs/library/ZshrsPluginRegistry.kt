package com.menketechnologies.zshrs.library

import com.google.gson.JsonParser
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.process.CapturingProcessHandler
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.Service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.AdditionalLibraryRootsListener
import com.intellij.openapi.util.SystemInfo
import com.intellij.util.concurrency.AppExecutorUtil
import com.menketechnologies.zshrs.ZshrsSettings
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * Project-scoped cache for the `zshrs --dump-plugins` output. The
 * `AdditionalLibraryRootsProvider` API is called on the indexer's
 * background thread under a read action — it CANNOT spawn a process
 * inline (would block indexing and trigger SlowOperations). This
 * service runs the dump out-of-band on `AppExecutorUtil`, atomically
 * publishes the result, and notifies the platform via
 * `AdditionalLibraryRootsListener` so the External Libraries node
 * re-renders.
 */
@Service(Service.Level.PROJECT)
class ZshrsPluginRegistry(private val project: Project) {

    private val cache = AtomicReference<List<PluginEntry>>(emptyList())
    private val refreshing = AtomicBoolean(false)
    private val initialized = AtomicBoolean(false)

    /** Latest snapshot. Empty until the first refresh completes. */
    fun snapshot(): List<PluginEntry> {
        // First reader triggers the initial async refresh — the provider
        // returns an empty list this round, then re-fires once results
        // land via AdditionalLibraryRootsListener.
        if (initialized.compareAndSet(false, true)) {
            refreshAsync()
        }
        return cache.get()
    }

    /**
     * Schedule a `zshrs --dump-plugins` invocation on the app executor.
     * Coalesces concurrent calls — second caller while a refresh is
     * in flight returns immediately.
     */
    fun refreshAsync() {
        if (!refreshing.compareAndSet(false, true)) return
        AppExecutorUtil.getAppExecutorService().submit {
            try {
                val fresh = runDump()
                val previous = cache.getAndSet(fresh)
                if (previous != fresh) {
                    notifyChange()
                }
            } catch (t: Throwable) {
                LOG.warn("plugin dump failed", t)
            } finally {
                refreshing.set(false)
            }
        }
    }

    private fun runDump(): List<PluginEntry> {
        val exe = resolveExe() ?: return emptyList()
        val cmd = GeneralCommandLine(exe, "--dump-plugins").withCharset(Charsets.UTF_8)
        val handler = CapturingProcessHandler(cmd)
        val out = handler.runProcess(10_000)
        if (out.exitCode != 0) {
            LOG.warn("zshrs --dump-plugins exit ${out.exitCode}: ${out.stderr.take(400)}")
            return emptyList()
        }
        return parse(out.stdout)
    }

    private fun parse(json: String): List<PluginEntry> = parsePluginDumpJson(json, LOG::warn)

    private fun notifyChange() {
        ApplicationManager.getApplication().invokeLater {
            project.messageBus
                .syncPublisher(AdditionalLibraryRootsListener.TOPIC)
                .libraryRootsChanged(LIBRARY_NODE, emptyList(), emptyList(), LIBRARY_NODE)
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

    companion object {
        private val LOG = Logger.getInstance(ZshrsPluginRegistry::class.java)
        const val LIBRARY_NODE = "zsh plugins"

        fun getInstance(project: Project): ZshrsPluginRegistry =
            project.getService(ZshrsPluginRegistry::class.java)
    }
}

/**
 * Parses the `zshrs --dump-plugins` JSON payload. Extracted as a
 * top-level function so it can be unit-tested without instantiating
 * the project-scoped registry service.
 *
 * Malformed input, missing keys, and unexpected types all map to an
 * empty list — the External Libraries view degrades gracefully when
 * a future zshrs version changes the schema.
 */
fun parsePluginDumpJson(json: String, onError: (String, Throwable) -> Unit = { _, _ -> }): List<PluginEntry> {
    return try {
        val root = JsonParser.parseString(json).asJsonObject
        val arr = root.getAsJsonArray("plugins") ?: return emptyList()
        arr.mapNotNull { el ->
            val o = el.asJsonObject
            PluginEntry(
                manager = o.get("manager")?.asString ?: return@mapNotNull null,
                name = o.get("name")?.asString ?: return@mapNotNull null,
                root = o.get("root")?.asString ?: return@mapNotNull null,
            )
        }
    } catch (e: Exception) {
        onError("plugin dump JSON parse failed", e)
        emptyList()
    }
}
