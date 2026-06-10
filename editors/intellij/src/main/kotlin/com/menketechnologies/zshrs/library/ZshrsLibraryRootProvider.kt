package com.menketechnologies.zshrs.library

import com.intellij.navigation.ItemPresentation
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.AdditionalLibraryRootsProvider
import com.intellij.openapi.roots.SyntheticLibrary
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFile
import com.menketechnologies.zshrs.ZshrsIcons
import javax.swing.Icon

/**
 * Populates *External Libraries* in the Project view with every zsh
 * plugin that zshrs has sourced. Data comes from
 * `ZshrsPluginRegistry.snapshot()` which mirrors the `plugins` table
 * in `~/.zshrs/plugins.db` (or `$ZSHRS_HOME/plugins.db`) grouped by
 * detected plugin manager: zinit / oh-my-zsh / prezto / antidote /
 * antigen / zplug / zsh-more-completions / zpwr / loose.
 *
 * Each entry becomes one navigable, indexable, completable library
 * root — go-to-definition, find-usages, semantic-tokens overlay,
 * rename, and the LSP function/alias completions all extend across
 * every sourced plugin without the user opening them as projects.
 */
class ZshrsLibraryRootProvider : AdditionalLibraryRootsProvider() {

    override fun getAdditionalProjectLibraries(project: Project): Collection<SyntheticLibrary> {
        val lfs = LocalFileSystem.getInstance()
        return ZshrsPluginRegistry.getInstance(project).snapshot()
            .mapNotNull { entry ->
                val vf = lfs.findFileByPath(entry.root) ?: return@mapNotNull null
                if (!vf.isDirectory) return@mapNotNull null
                ZshrsPluginLibrary(entry, vf)
            }
    }

    override fun getRootsToWatch(project: Project): Collection<VirtualFile> {
        val lfs = LocalFileSystem.getInstance()
        return ZshrsPluginRegistry.getInstance(project).snapshot()
            .mapNotNull { lfs.findFileByPath(it.root) }
    }
}

/**
 * One synthetic library per plugin.
 *
 * `comparisonId` ("zshrs:<manager>:<name>") is stable across IDE
 * restarts so the platform can intern roots between sessions. The
 * platform's External Libraries tree renderer detects `ItemPresentation`
 * via `instanceof` and uses it for the node label / location / icon —
 * SyntheticLibrary itself exposes no presentation hook, so the right
 * pattern is to extend SyntheticLibrary AND implement ItemPresentation
 * on the same class.
 *
 * Equality keys on `(manager, name, root)` so the platform's
 * library-cache invalidator fires only when the dump actually changes
 * — not on every read.
 */
private class ZshrsPluginLibrary(
    val entry: PluginEntry,
    private val rootDir: VirtualFile,
) : SyntheticLibrary("zshrs:${entry.manager}:${entry.name}", null), ItemPresentation {

    override fun getSourceRoots(): Collection<VirtualFile> = listOf(rootDir)
    override fun getBinaryRoots(): Collection<VirtualFile> = emptyList()

    override fun getPresentableText(): String = "${entry.name} (${entry.manager})"
    override fun getLocationString(): String = entry.root
    override fun getIcon(unused: Boolean): Icon = ZshrsIcons.FILE

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is ZshrsPluginLibrary) return false
        return entry == other.entry
    }
    override fun hashCode(): Int = entry.hashCode()
}
