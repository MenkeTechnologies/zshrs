package com.menketechnologies.zshrs

import org.junit.Assert.*
import org.junit.Test

/**
 * Pure-logic tests for [ZshrsSettings]. The `getInstance()` path
 * touches the IntelliJ ApplicationManager and must run under
 * `gradle :test` (BasePlatformTestCase); but the small parsers
 * (`supportedExtensions`, `isSupportedFile`) are pure on a freshly-
 * constructed instance and can be tested with plain JUnit.
 */
class ZshrsSettingsTest {

    private fun fresh(ext: String): ZshrsSettings {
        val s = ZshrsSettings()
        s.fileExtensions = ext
        return s
    }

    @Test fun `default extension set contains zsh`() {
        val s = fresh("zsh")
        assertTrue("zsh" in s.supportedExtensions())
    }

    @Test fun `supportedExtensions parses comma list`() {
        val s = fresh("zsh, sh,bash; pl")
        val got = s.supportedExtensions().toSet()
        assertEquals(setOf("zsh", "sh", "bash", "pl"), got)
    }

    @Test fun `supportedExtensions strips leading dots`() {
        val s = fresh(".zsh, .sh")
        assertEquals(listOf("zsh", "sh"), s.supportedExtensions())
    }

    @Test fun `supportedExtensions ignores blanks`() {
        val s = fresh("  ,  zsh ,,, ")
        assertEquals(listOf("zsh"), s.supportedExtensions())
    }

    @Test fun `isSupportedFile matches by extension`() {
        val s = fresh("zsh")
        assertTrue(s.isSupportedFile("plugin.zsh", "zsh"))
        assertFalse(s.isSupportedFile("readme.md", "md"))
    }

    @Test fun `isSupportedFile recognizes dotfile names without extension`() {
        val s = fresh("zsh")
        for (name in listOf(".zshrc", ".zshenv", ".zlogin", ".zlogout", ".zprofile", ".zpreztorc")) {
            assertTrue("$name should be supported", s.isSupportedFile(name, null))
        }
    }

    @Test fun `isSupportedFile rejects unrelated dotfiles`() {
        val s = fresh("zsh")
        assertFalse(s.isSupportedFile(".bashrc", null))
        assertFalse(s.isSupportedFile(".profile", null))
    }
}
