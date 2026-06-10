package com.menketechnologies.zshrs.library

/**
 * Single plugin entry parsed from `zshrs --dump-plugins` JSON. One entry =
 * one library node under External Libraries.
 */
data class PluginEntry(
    val manager: String,
    val name: String,
    val root: String,
)
