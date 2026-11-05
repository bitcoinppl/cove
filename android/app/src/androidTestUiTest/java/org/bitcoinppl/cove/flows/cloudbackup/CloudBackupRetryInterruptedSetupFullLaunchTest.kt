package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
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
     * The provider rejects the setup's wallet upload after accepting its master key, and the
     * production Enable action retries with the passkey the failed attempt created
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

        onboarding.retryCloudBackupEnable().assertCloudBackupSuccess()

        assertEquals(
            "retrying the failed setup must reuse its passkey",
            1,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        // the background verifier may upload again while the cloud hides the writes, so only the
        // retry's own upload is certain
        assertTrue(
            "retrying the failed setup must upload the wallet again",
            ScriptedCloudStorageAccess.uploadCount(UploadKind.WALLET) >= 2,
        )
    }
}
