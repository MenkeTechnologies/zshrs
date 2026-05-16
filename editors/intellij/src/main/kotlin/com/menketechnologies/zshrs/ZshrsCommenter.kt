package com.menketechnologies.zshrs

import com.intellij.lang.Commenter

/**
 * zsh line comments use `#`. There is no native block-comment form, so the
 * block prefix/suffix wrap the selection in a `: <<'###BLOCK###'` here-doc
 * — same trick zsh users employ to block-out code. Cmd+/, Cmd+Opt+/.
 */
class ZshrsCommenter : Commenter {
    override fun getLineCommentPrefix(): String = "# "
    override fun getBlockCommentPrefix(): String = ": <<'###BLOCK###'\n"
    override fun getBlockCommentSuffix(): String = "\n###BLOCK###\n"
    override fun getCommentedBlockCommentPrefix(): String = ": <<'###BLOCK###'\n"
    override fun getCommentedBlockCommentSuffix(): String = "\n###BLOCK###\n"
}
