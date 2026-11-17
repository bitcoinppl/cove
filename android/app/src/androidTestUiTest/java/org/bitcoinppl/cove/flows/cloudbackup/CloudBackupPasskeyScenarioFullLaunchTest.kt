package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.RustConnectivityManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupPasskeyScenarioFullLaunchTest {
    private companion object {
        const val EXPECTED_DISCOVERY_ATTEMPTS = 2
        const val PASSKEY_RETRY_TIMEOUT_MS = 30_000L
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    fun prePresentationDiscoveryFailureRetriesOnceThenSucceeds() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        ScriptedPasskeyProvider.configureResults(
            Invocation.DISCOVER,
            Result.PRE_PRESENTATION_FAILURE,
            Result.SUCCESS,
        )
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.tap(UiTarget.Text("Restore with Passkey"))

        assertTrue(
            "expected one bounded discovery retry to reach the scripted success; " +
                "discover=${ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)}, " +
                "authenticate=${ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE)}, " +
                "create=${ScriptedPasskeyProvider.callCount(Invocation.CREATE)}, " +
                "connectivity=${rustConnectivityStatus()}, " +
                "masterDownloads=${ScriptedCloudStorageAccess.masterDownloadCount()}, " +
                "walletLists=${ScriptedCloudStorageAccess.walletListCount()}, " +
                "walletDownloads=${ScriptedCloudStorageAccess.walletDownloadCount()}",
            ScriptedPasskeyProvider.awaitCallCount(
                Invocation.DISCOVER,
                expected = EXPECTED_DISCOVERY_ATTEMPTS,
                timeoutMs = PASSKEY_RETRY_TIMEOUT_MS,
            ),
        )
        assertEquals(
            EXPECTED_DISCOVERY_ATTEMPTS,
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER),
        )
        assertEquals(
            "discovery recovery must not fall through to registration",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        assertEquals(
            "the discovered credential and PRF result should complete the passkey operation",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE),
        )
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000,
    ) {
        if (!wait(Until.hasObject(By.text(value)), timeoutMs)) error("Timed out waiting for text \"$value\"")
    }

    private fun rustConnectivityStatus() =
        RustConnectivityManager().use { manager ->
            manager.state().status
        }
}
