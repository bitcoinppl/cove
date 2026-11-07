package org.bitcoinppl.cove

import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SmallTest
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * KeyTeleport links and shared text stay off in this build: Android resolves neither a tapped
 * keyteleport.com link nor text shared from another app to Cove
 *
 * The manifest's `KeyTeleportLinkActivity` alias holds both intent filters, and every flavour
 * builds it disabled through the `keyTeleportAppLinksEnabled` placeholder in
 * `android/app/build.gradle.kts`, so a receiver response reaches the app only by scan or paste.
 * Shipping the alias enabled by accident fails this test
 */
@SmallTest
@RunWith(AndroidJUnit4::class)
class KeyTeleportLinksDisabledTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val packageManager = context.packageManager

    @Test
    fun keyTeleportLinksAndSharedTextResolveToNoCoveActivity() {
        // the launcher entry resolves, so an empty answer below means the filters are off, not
        // that the query can't see this package
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER).setPackage(context.packageName)
        assertTrue("the launcher activity should resolve", resolvesInCove(launcher).isNotEmpty())

        val link =
            Intent(Intent.ACTION_VIEW, Uri.parse("https://keyteleport.com/#B\$2S0100EXAMPLE"))
                .addCategory(Intent.CATEGORY_BROWSABLE)
        val sharedText = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, "B\$2S0100EXAMPLE")

        assertEquals("a keyteleport.com link must not open Cove", emptyList<String>(), resolvesInCove(link))
        assertEquals("text shared from another app must not open Cove", emptyList<String>(), resolvesInCove(sharedText))
    }

    /** The Cove activities Android would offer for the intent, by class name */
    private fun resolvesInCove(intent: Intent): List<String> =
        packageManager
            .queryIntentActivities(intent.setPackage(context.packageName), PackageManager.ResolveInfoFlags.of(0))
            .map { it.activityInfo.name }
}
