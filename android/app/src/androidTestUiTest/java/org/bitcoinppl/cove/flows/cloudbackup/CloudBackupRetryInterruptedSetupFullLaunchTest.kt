package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.UploadKind
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupRetryInterruptedSetupFullLaunchTest {
    private companion object {
        const val UPLOAD_FAILURE_MESSAGE = "Cove couldn't complete this cloud backup request. Please try again."
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    /**
     * The provider rejects the setup's wallet upload after accepting its master key, which leaves
     * the saved passkey's pause, and Confirm Passkey retries with the passkey, master key and
     * namespace the failed attempt staged, leaving one backup in the cloud
     */
    @Test
    fun failedSetupUploadRetriesWithoutNewPasskey() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        ScriptedCloudStorageAccess.failUpload(UploadKind.WALLET, attempt = 1)
        launchFullApp()

        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        val onboarding =
            FullLaunchOnboardingRobot(device)
                .tapGetStarted()
                .chooseNewUser()
                .openCloudBackupFromBackupWallet()
                .assertCloudBackupDetails()
                .enableCloudBackupFromDetailsOnce()

        assertTrue("expected test passkey creation", ScriptedPasskeyProvider.awaitCreation())
        assertTrue(
            "expected the provider to reject the first wallet upload",
            ScriptedCloudStorageAccess.awaitUploadCount(UploadKind.WALLET, expected = 1),
        )
        onboarding.assertCloudBackupEnableFailed(UPLOAD_FAILURE_MESSAGE)
        assertEquals(1, ScriptedPasskeyProvider.callCount(Invocation.CREATE))

        val failedAttempt = ScriptedCloudStorageAccess.uploads()
        val failedWallet = failedAttempt.last { it.kind == UploadKind.WALLET }
        assertFalse(failedWallet.accepted)
        val namespace = failedWallet.namespace
        val walletRecordId = requireNotNull(failedWallet.recordId)
        assertEquals(
            "the failed attempt's master key went to another namespace",
            listOf(namespace),
            failedAttempt.filter { it.kind == UploadKind.MASTER }.map { it.namespace },
        )

        // the failed upload leaves the passkey's pause, whose Confirm is the retry
        onboarding.assertPendingEnableRecovery()
        val failedAssertions = passkeyAssertionCount()

        // the enable is still running while this upload is held, so no background work made it
        ScriptedCloudStorageAccess.blockNextWalletUpload()
        try {
            onboarding.confirmSavedPasskeyAfterFailure()
            assertTrue(
                "expected the retry to upload the wallet",
                ScriptedCloudStorageAccess.awaitBlockedWalletUpload(),
            )
            assertNull(device.findObject(By.text("Cloud Backup enabled")))
            val retried = ScriptedCloudStorageAccess.uploads().last { it.kind == UploadKind.WALLET }
            assertEquals("the retry uploaded the wallet to a new namespace", namespace, retried.namespace)
            assertEquals("the retry uploaded another wallet record", walletRecordId, retried.recordId)
            assertTrue(retried.accepted)
        } finally {
            ScriptedCloudStorageAccess.releaseBlockedWalletUpload()
        }

        onboarding.assertCloudBackupSuccess()

        // repeated writes to the same records are fine, a second namespace or record is not
        assertEquals(
            "retrying the failed setup must reuse its passkey",
            1,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        assertTrue("the retry did not ask for the saved passkey", passkeyAssertionCount() > failedAssertions)
        assertEquals(
            "an upload went to another namespace",
            setOf(namespace),
            ScriptedCloudStorageAccess.uploads().map { it.namespace }.toSet(),
        )
        assertEquals(listOf(namespace), ScriptedCloudStorageAccess.freshEnableNamespaces())
        assertEquals(
            listOf(walletRecordId),
            ScriptedCloudStorageAccess.freshEnableWalletRecordIds(namespace),
        )
    }

    private fun passkeyAssertionCount(): Int =
        ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE) +
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)
}
