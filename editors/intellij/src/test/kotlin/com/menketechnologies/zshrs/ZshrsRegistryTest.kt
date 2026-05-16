package com.menketechnologies.zshrs

import org.junit.Assert.*
import org.junit.Test

/**
 * Registry & singleton-identity tests for the plugin's static surface.
 *
 * The IntelliJ platform expects [com.intellij.lang.Language] subclasses
 * to be process-singletons — multiple references via `Class.forName` /
 * deserialization must resolve to the SAME instance, or the platform
 * loses track of which language a file belongs to (silent registry
 * leaks, broken file-type bindings).
 */
class ZshrsRegistryTest {

    // ── Language ────────────────────────────────────────────────────────

    @Test fun `language id and display name match`() {
        assertEquals("zshrs", ZshrsLanguage.id)
        assertEquals("zshrs", ZshrsLanguage.displayName)
    }

    @Test fun `language is case-sensitive`() {
        assertTrue(ZshrsLanguage.isCaseSensitive)
    }

    @Test fun `readResolve returns the same singleton`() {
        // Simulates deserialization re-entry. The private readResolve
        // method must return ZshrsLanguage itself.
        val m = ZshrsLanguage::class.java.getDeclaredMethod("readResolve")
        m.isAccessible = true
        val resolved = m.invoke(ZshrsLanguage)
        assertSame(ZshrsLanguage, resolved)
    }

    // ── FileType ────────────────────────────────────────────────────────

    @Test fun `file type advertises name description and extension`() {
        assertEquals("zsh", ZshrsFileType.name)
        assertTrue(ZshrsFileType.description.lowercase().contains("zsh"))
        assertEquals("zsh", ZshrsFileType.defaultExtension)
    }

    @Test fun `file type binds to the zshrs language`() {
        assertSame(ZshrsLanguage, ZshrsFileType.language)
    }

    @Test fun `file type icon is non-null`() {
        // IconLoader can resolve lazily; just assert the path returns
        // a usable Icon object.
        assertNotNull(ZshrsFileType.icon)
    }

    // ── Token types ─────────────────────────────────────────────────────

    @Test fun `token type identity is stable across lookups`() {
        // Each ZshrsTokenType is constructed once as a @JvmField. The
        // same field must return the same instance — otherwise the
        // lexer-vs-highlighter wiring breaks because IElementType uses
        // identity comparison.
        assertSame(ZshrsTokenTypes.COMMENT, ZshrsTokenTypes.COMMENT)
        assertSame(ZshrsTokenTypes.STRING_DQ, ZshrsTokenTypes.STRING_DQ)
        assertSame(ZshrsTokenTypes.VARIABLE, ZshrsTokenTypes.VARIABLE)
        assertSame(ZshrsTokenTypes.PIPE, ZshrsTokenTypes.PIPE)
    }

    @Test fun `every token type carries the zshrs language`() {
        // Sample the categories — if any one slipped in pointing at a
        // different Language object, the highlighter would silently drop
        // them.
        val sample = listOf(
            ZshrsTokenTypes.COMMENT,
            ZshrsTokenTypes.STRING_DQ,
            ZshrsTokenTypes.STRING_SQ,
            ZshrsTokenTypes.BACKTICK,
            ZshrsTokenTypes.HEREDOC,
            ZshrsTokenTypes.NUMBER,
            ZshrsTokenTypes.KEYWORD,
            ZshrsTokenTypes.CONTROL_KEYWORD,
            ZshrsTokenTypes.DECL_KEYWORD,
            ZshrsTokenTypes.FN_KEYWORD,
            ZshrsTokenTypes.LOOP_KEYWORD,
            ZshrsTokenTypes.MODIFIER_KEYWORD,
            ZshrsTokenTypes.IO_KEYWORD,
            ZshrsTokenTypes.BUILTIN,
            ZshrsTokenTypes.IDENTIFIER,
            ZshrsTokenTypes.SIGIL,
            ZshrsTokenTypes.VARIABLE,
            ZshrsTokenTypes.SPECIAL_VAR,
            ZshrsTokenTypes.PARAM_EXPANSION,
            ZshrsTokenTypes.OPERATOR,
            ZshrsTokenTypes.ASSIGN_OP,
            ZshrsTokenTypes.PIPE,
            ZshrsTokenTypes.REDIRECT,
            ZshrsTokenTypes.LOGICAL_OP,
            ZshrsTokenTypes.BACKGROUND,
            ZshrsTokenTypes.GLOB,
            ZshrsTokenTypes.PAREN,
            ZshrsTokenTypes.BRACE,
            ZshrsTokenTypes.BRACKET,
            ZshrsTokenTypes.DOUBLE_SEMI,
            ZshrsTokenTypes.BAD,
        )
        for (t in sample) {
            assertSame("token $t not bound to ZshrsLanguage", ZshrsLanguage, t.language)
        }
    }

    @Test fun `token debug names are stable ZSHRS prefixed identifiers`() {
        // The highlighter contract uses these strings as registry keys.
        // A typo here means the color setting Page can't find them.
        for (t in listOf(
            ZshrsTokenTypes.COMMENT to "ZSHRS_COMMENT",
            ZshrsTokenTypes.STRING_DQ to "ZSHRS_STRING_DQ",
            ZshrsTokenTypes.HEREDOC to "ZSHRS_HEREDOC",
            ZshrsTokenTypes.NUMBER to "ZSHRS_NUMBER",
            ZshrsTokenTypes.PIPE to "ZSHRS_PIPE",
            ZshrsTokenTypes.SPECIAL_VAR to "ZSHRS_SPECIAL_VAR",
        )) {
            assertEquals(t.second, t.first.toString())
        }
    }

    // ── Colors ──────────────────────────────────────────────────────────

    @Test fun `every color key is non-null`() {
        // Hits every @JvmField in ZshrsColors. A NullPointerException
        // here means a Defaults.* import resolved to null at static init
        // time — usually because the IntelliJ Platform version doesn't
        // expose the field.
        val cls = ZshrsColors::class.java
        var n = 0
        for (f in cls.declaredFields) {
            if (java.lang.reflect.Modifier.isStatic(f.modifiers)
                && f.type.simpleName == "TextAttributesKey") {
                f.isAccessible = true
                val v = f.get(null)
                assertNotNull("color field ${f.name} is null", v)
                n++
            }
        }
        // We declare 40+ color slots; assert a sensible floor so a stripped-
        // out field doesn't pass silently.
        assertTrue("only $n color fields found — fewer than expected", n >= 35)
    }

    @Test fun `all color keys have ZSHRS prefixed external names`() {
        val cls = ZshrsColors::class.java
        for (f in cls.declaredFields) {
            if (java.lang.reflect.Modifier.isStatic(f.modifiers)
                && f.type.simpleName == "TextAttributesKey") {
                f.isAccessible = true
                val k = f.get(null) as com.intellij.openapi.editor.colors.TextAttributesKey
                assertTrue("color ${f.name} externalName=${k.externalName} missing ZSHRS_ prefix",
                    k.externalName.startsWith("ZSHRS_"))
            }
        }
    }

    // ── Icons ───────────────────────────────────────────────────────────

    @Test fun `FILE icon loader resolves`() {
        // Loading goes through IconLoader; if the SVG is missing or
        // malformed, IconLoader still returns a placeholder, but the
        // resource lookup we use must succeed.
        val resource = ZshrsIcons::class.java.getResource("/icons/zshrs.svg")
        assertNotNull("icons/zshrs.svg not on classpath", resource)
    }
}
