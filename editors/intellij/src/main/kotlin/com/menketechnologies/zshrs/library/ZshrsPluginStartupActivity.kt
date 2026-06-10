package com.menketechnologies.zshrs.library

import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity

/**
 * Triggers the first `zshrs --dump-plugins` fetch at project open so
 * the External Libraries node is populated by the time the user
 * expands it — without this, the first call from the indexer races
 * with the async refresh and the user sees an empty External Libraries
 * tree for a few hundred milliseconds.
 */
class ZshrsPluginStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        ZshrsPluginRegistry.getInstance(project).refreshAsync()
    }
}
