package com.menketechnologies.zshrs

import org.junit.Assert.*
import org.junit.Test
import javax.xml.parsers.DocumentBuilderFactory

/**
 * Catches the most common plugin-loading failure mode: `plugin.xml`
 * references a class that doesn't exist (typo, rename, deleted file).
 * Every `implementation=` / `class=` / `factoryClass=` /
 * `serviceImplementation=` / `instance=` attribute must resolve to a
 * loadable JVM class on this module's classpath at test time.
 */
class ZshrsPluginManifestTest {

    @Test fun `every class referenced in plugin xml is loadable`() {
        val stream = javaClass.classLoader.getResourceAsStream("META-INF/plugin.xml")
            ?: fail("META-INF/plugin.xml not on classpath").let { return }
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(stream)
        val xpath = javax.xml.xpath.XPathFactory.newInstance().newXPath()
        val attrNames = listOf(
            "implementationClass", "implementation", "class",
            "factoryClass", "serviceImplementation", "instance",
        )
        val seen = mutableSetOf<String>()
        for (attr in attrNames) {
            val nodes = xpath.evaluate(
                "//@$attr", doc, javax.xml.xpath.XPathConstants.NODESET
            ) as org.w3c.dom.NodeList
            for (i in 0 until nodes.length) {
                seen += nodes.item(i).nodeValue
            }
        }
        // The icon class is referenced via a token (`X.FILE`); skip those non-FQ refs.
        val classFqns = seen.filter { it.contains('.') && !it.endsWith(".FILE") }
        assertTrue("no classes found in plugin.xml — XPath broken?", classFqns.isNotEmpty())
        for (fqn in classFqns) {
            try {
                Class.forName(fqn)
            } catch (t: ClassNotFoundException) {
                fail("plugin.xml references missing class: $fqn")
            }
        }
    }

    @Test fun `plugin xml declares zsh file type with the expected language`() {
        val stream = javaClass.classLoader.getResourceAsStream("META-INF/plugin.xml")
            ?: fail("META-INF/plugin.xml not on classpath").let { return }
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(stream)
        val xpath = javax.xml.xpath.XPathFactory.newInstance().newXPath()
        val lang = xpath.evaluate("//fileType/@language", doc) as String
        assertEquals("zshrs", lang)
        val exts = xpath.evaluate("//fileType/@extensions", doc) as String
        assertTrue("expected `zsh` in extensions: $exts", exts.contains("zsh"))
        val files = xpath.evaluate("//fileType/@fileNames", doc) as String
        for (dot in listOf(".zshrc", ".zshenv", ".zlogin", ".zlogout", ".zprofile")) {
            assertTrue("$dot missing from fileNames: $files", files.contains(dot))
        }
    }

    @Test fun `plugin xml registers a tool window for zshrs`() {
        val stream = javaClass.classLoader.getResourceAsStream("META-INF/plugin.xml")
            ?: fail("META-INF/plugin.xml not on classpath").let { return }
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(stream)
        val xpath = javax.xml.xpath.XPathFactory.newInstance().newXPath()
        val id = xpath.evaluate("//toolWindow/@id", doc) as String
        assertEquals("zshrs", id)
        val factory = xpath.evaluate("//toolWindow/@factoryClass", doc) as String
        assertTrue(factory.endsWith("ZshrsReflectionToolWindowFactory"))
    }
}
