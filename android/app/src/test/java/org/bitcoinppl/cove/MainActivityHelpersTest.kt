package org.bitcoinppl.cove

import android.content.Intent
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import org.junit.Test

class MainActivityHelpersTest {
    @Test
    fun catastrophicRestoreBackupFoundHasNoFailureMessage() {
        assertEquals(null, CatastrophicCloudRestoreResult.BackupFound.failureMessage)
    }

    @Test
    fun catastrophicRestoreAuthorizationErrorIsUserVisible() {
        val message = CatastrophicCloudRestoreResult.Inconclusive("wrong google drive account")
            .failureMessage

        assertEquals("wrong google drive account", message)
    }

    @Test
    fun catastrophicRestoreUnreadableBackupErrorIsUserVisible() {
        val message = CatastrophicCloudRestoreResult.Unreadable(
            "Cloud Backup data could not be read: master key backup is unreadable",
        ).failureMessage

        assertEquals("Cloud Backup data could not be read: master key backup is unreadable", message)
    }

    @Test
    fun actionViewRequiresTheEnabledKeyTeleportAlias() {
        assertTrue(
            shouldAcceptExternalKeyTeleportIntent(
                action = Intent.ACTION_VIEW,
                componentClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasEnabled = true,
            ),
        )
        assertFalse(
            shouldAcceptExternalKeyTeleportIntent(
                action = Intent.ACTION_VIEW,
                componentClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasEnabled = false,
            ),
        )
        assertFalse(
            shouldAcceptExternalKeyTeleportIntent(
                action = Intent.ACTION_VIEW,
                componentClassName = MainActivity::class.java.name,
                keyTeleportAliasClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasEnabled = true,
            ),
        )
    }

    @Test
    fun actionSendRequiresTheKeyTeleportEntryGate() {
        assertFalse(
            shouldAcceptExternalKeyTeleportIntent(
                action = Intent.ACTION_SEND,
                componentClassName = MainActivity::class.java.name,
                keyTeleportAliasClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasEnabled = false,
            ),
        )
        assertTrue(
            shouldAcceptExternalKeyTeleportIntent(
                action = Intent.ACTION_SEND,
                componentClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasClassName = KEY_TELEPORT_ALIAS,
                keyTeleportAliasEnabled = true,
            ),
        )
    }

    @Test
    fun repeatedExternalLinkIsSkippedOnlyWhileItsFlowIsAlive() {
        assertTrue(
            shouldSkipHandledExternalIntent(
                signatureMatches = true,
                hasActiveKeyTeleportFlow = true,
            ),
        )
        assertFalse(
            shouldSkipHandledExternalIntent(
                signatureMatches = true,
                hasActiveKeyTeleportFlow = false,
            ),
        )
        assertFalse(
            shouldSkipHandledExternalIntent(
                signatureMatches = false,
                hasActiveKeyTeleportFlow = true,
            ),
        )
    }

    private companion object {
        const val KEY_TELEPORT_ALIAS = "org.bitcoinppl.cove.KeyTeleportLinkActivity"
    }
}
