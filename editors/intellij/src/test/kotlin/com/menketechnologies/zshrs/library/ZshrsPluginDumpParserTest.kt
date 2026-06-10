package com.menketechnologies.zshrs.library

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Round-trips the JSON shape emitted by `zshrs --dump-plugins` through
 * the parser used by `ZshrsLibraryRootProvider`. Pins the schema —
 * any future zshrs-side rename of `manager` / `name` / `root` must
 * land alongside a parser update.
 */
class ZshrsPluginDumpParserTest {

    @Test fun `parses canonical dump with mixed managers`() {
        val json = """
            {"schema":1,"plugins":[
              {"manager":"zinit","name":"zsh-users/zsh-autosuggestions",
               "root":"/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions"},
              {"manager":"oh-my-zsh","name":"git",
               "root":"/Users/wizard/.oh-my-zsh/plugins/git"},
              {"manager":"loose","name":"something",
               "root":"/opt/local/share/zsh/something"}
            ]}
        """.trimIndent()
        val entries = parsePluginDumpJson(json)
        assertEquals(3, entries.size)
        assertEquals(
            PluginEntry("zinit", "zsh-users/zsh-autosuggestions",
                "/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions"),
            entries[0],
        )
        assertEquals(
            PluginEntry("oh-my-zsh", "git", "/Users/wizard/.oh-my-zsh/plugins/git"),
            entries[1],
        )
        assertEquals(
            PluginEntry("loose", "something", "/opt/local/share/zsh/something"),
            entries[2],
        )
    }

    @Test fun `empty plugins array yields empty list`() {
        assertTrue(parsePluginDumpJson("""{"schema":1,"plugins":[]}""").isEmpty())
    }

    @Test fun `entries with missing required field are dropped`() {
        val json = """
            {"schema":1,"plugins":[
              {"manager":"zinit","name":"good","root":"/a"},
              {"manager":"zinit","name":"missing-root"},
              {"manager":"zinit","root":"/b"},
              {"name":"no-manager","root":"/c"}
            ]}
        """.trimIndent()
        val entries = parsePluginDumpJson(json)
        assertEquals(1, entries.size)
        assertEquals("good", entries[0].name)
    }

    @Test fun `malformed json returns empty list and reports error`() {
        var captured: Pair<String, Throwable>? = null
        val out = parsePluginDumpJson("{not json") { msg, t -> captured = msg to t }
        assertTrue(out.isEmpty())
        assertTrue(captured != null)
        assertTrue(captured!!.first.contains("parse failed"))
    }

    @Test fun `missing plugins key yields empty list`() {
        assertTrue(parsePluginDumpJson("""{"schema":1}""").isEmpty())
    }
}
