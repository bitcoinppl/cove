package org.bitcoinppl.cove.cloudbackup

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidBackupRulesTest {
    @Test
    fun backupRulesExcludeAppDataAndSecurePrefs() {
        val excludes = readExcludeDomains(resourceXml("backup_rules.xml"))

        assertEquals(setOf("database", "file", "sharedpref", "external"), excludes["full-backup-content"])
        assertTrue(excludes["full-backup-content"].orEmpty().contains("sharedpref"))
    }

    @Test
    fun dataExtractionRulesExcludeAppDataAndSecurePrefsForBackupAndTransfer() {
        val excludes = readExcludeDomains(resourceXml("data_extraction_rules.xml"))
        val expected = setOf("database", "file", "sharedpref", "external")

        assertEquals(expected, excludes["cloud-backup"])
        assertEquals(expected, excludes["device-transfer"])
        assertTrue(excludes["cloud-backup"].orEmpty().contains("sharedpref"))
        assertTrue(excludes["device-transfer"].orEmpty().contains("sharedpref"))
    }

    private fun readExcludeDomains(file: File): Map<String, Set<String>> {
        val document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val root = document.documentElement
        if (root.tagName == "full-backup-content") {
            return mapOf(root.tagName to excludeDomains(root))
        }

        val result = mutableMapOf<String, Set<String>>()
        for (index in 0 until root.childNodes.length) {
            val node = root.childNodes.item(index)
            if (node !is org.w3c.dom.Element) continue

            result[node.tagName] = excludeDomains(node)
        }

        return result
    }

    private fun resourceXml(name: String): File =
        listOf(
            File("src/main/res/xml/$name"),
            File("app/src/main/res/xml/$name"),
            File("android/app/src/main/res/xml/$name"),
        ).first(File::exists)

    private fun excludeDomains(parent: org.w3c.dom.Element): Set<String> {
        val domains = mutableSetOf<String>()
        for (index in 0 until parent.childNodes.length) {
            val node = parent.childNodes.item(index)
            if (node !is org.w3c.dom.Element || node.tagName != "exclude") continue

            domains.add(node.getAttribute("domain"))
        }

        return domains
    }
}
