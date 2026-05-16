package com.menketechnologies.zshrs

import org.junit.Assert.*
import org.junit.Test

/**
 * The Commenter contract is what IntelliJ uses for Cmd+/ and
 * Cmd+Opt+/. Wrong prefixes here mean the editor's comment shortcuts
 * silently produce broken zsh.
 */
class ZshrsCommenterTest {
    private val c = ZshrsCommenter()

    @Test fun `line comment prefix is hash-space`() {
        assertEquals("# ", c.lineCommentPrefix)
    }

    @Test fun `block-comment prefix opens a quoted heredoc`() {
        // Quoted `'###BLOCK###'` keeps zsh from expanding anything in the body
        assertTrue(c.blockCommentPrefix!!.contains("<<'###BLOCK###'"))
        assertTrue(c.blockCommentSuffix!!.contains("###BLOCK###"))
    }

    @Test fun `commented-block markers match`() {
        assertEquals(c.blockCommentPrefix, c.commentedBlockCommentPrefix)
        assertEquals(c.blockCommentSuffix, c.commentedBlockCommentSuffix)
    }
}
