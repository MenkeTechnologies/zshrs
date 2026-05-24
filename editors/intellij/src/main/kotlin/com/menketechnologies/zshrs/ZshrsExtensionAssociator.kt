package com.menketechnologies.zshrs

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.fileTypes.FileTypeManager
import com.intellij.openapi.fileTypes.ex.FileTypeManagerEx
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity

/**
 * Applies the dynamic file-extension association for zshrs at startup
 * and re-applies on demand after a Settings save.
 *
 * Two sources of additional extensions:
 *   1. `claimShFiles` checkbox — opt-in `.sh` claim. Off by default
 *      because users who write POSIX-only / bash-only scripts in `.sh`
 *      files don't want zshrs colorization forced on them.
 *   2. `fileExtensions` text field — comma-separated user override
 *      (e.g. `zsh, sh, bash, ksh`) for power users who want to claim a
 *      custom set without code changes.
 *
 * Static `.zsh` association lives in `plugin.xml`; this activity only
 * ADDS extensions on top of that base. Removing an extension here
 * persists across IDE restarts via `FileTypeManager`'s own state.
 */
class ZshrsExtensionAssociator : ProjectActivity {
    override suspend fun execute(project: Project) {
        applyDynamicAssociations()
    }

    companion object {
        /**
         * Apply (or re-apply) the dynamic-extension list. Idempotent —
         * `associateExtension` does nothing if the extension is already
         * mapped to the same file type. Safe to call from the Settings
         * dialog's `apply()` after the user changes the toggles.
         */
        fun applyDynamicAssociations() {
            val s = ZshrsSettings.getInstance()
            val wanted = buildSet {
                add("zsh")  // baseline; harmless duplicate of plugin.xml
                if (s.claimShFiles) add("sh")
                s.supportedExtensions().forEach { add(it) }
            }
            // Must run on the write thread because FileTypeManager
            // mutates a global registry. `invokeAndWait` keeps the
            // call site simple (settings save returns synchronously
            // after the new associations are live).
            val app = ApplicationManager.getApplication()
            val task = Runnable {
                app.runWriteAction {
                    val mgr = FileTypeManager.getInstance() as FileTypeManagerEx
                    for (ext in wanted) {
                        mgr.associateExtension(ZshrsFileType, ext)
                    }
                }
            }
            if (app.isDispatchThread) task.run() else app.invokeAndWait(task)
        }
    }
}
