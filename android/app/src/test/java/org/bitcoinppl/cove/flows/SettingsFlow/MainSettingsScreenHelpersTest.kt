package org.bitcoinppl.cove.flows.SettingsFlow

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MainSettingsScreenHelpersTest {
    @Test
    fun cloudBackupSettingsStayHiddenInDecoyMode() {
        assertTrue(shouldShowCloudBackupSettings(isInDecoyMode = false))
        assertFalse(shouldShowCloudBackupSettings(isInDecoyMode = true))
    }
}
