package org.bitcoinppl.cove.flows.onboarding

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.NamespaceResult
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class OnboardingRecoveryScenarioFullLaunchTest {
    private companion object {
        const val FINAL_NAMESPACE_REQUEST_COUNT = 3
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
    }

    @Test
    fun emptyThenEmptyThenBackupFoundAfterCheckAgain() {
        ScriptedCloudStorageAccess.configureNamespaceResults(
            NamespaceResult.EMPTY,
            NamespaceResult.EMPTY,
            NamespaceResult.BACKUP_FOUND,
            blockedRequest = 2,
        )
        launchFullApp()

        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        assertTrue(
            "expected the silent startup inventory",
            ScriptedCloudStorageAccess.awaitNamespaceRequestCount(1),
        )

        FullLaunchOnboardingRobot(device).tapGetStarted()
        device.waitForResource("onboarding.bitcoinChoice.restore").click()
        device.waitForText("Nothing visible yet")

        device.waitForText("Check Again").click()
        assertTrue(
            "expected Check Again to start the second inventory",
            ScriptedCloudStorageAccess.awaitNamespaceRequestCount(2),
        )
        device.waitForText("Looking for Google Drive backup...")

        ScriptedCloudStorageAccess.releaseBlockedNamespaceRequest()
        device.waitForText("Nothing visible yet")
        device.waitForText("Check Again").click()

        assertTrue(
            "expected the third inventory to find the backup",
            ScriptedCloudStorageAccess.awaitNamespaceRequestCount(FINAL_NAMESPACE_REQUEST_COUNT),
        )
        device.waitForText("Google Drive Backup Found")

        assertEquals(
            listOf(
                CloudAccessPolicy.SILENT,
                CloudAccessPolicy.SILENT,
                CloudAccessPolicy.SILENT,
            ),
            ScriptedCloudStorageAccess.namespaceRequestPolicies(),
        )
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000,
    ) = wait(Until.findObject(By.text(value)), timeoutMs)
        ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.waitForResource(
        value: String,
        timeoutMs: Long = 20_000,
    ) = wait(Until.findObject(By.res(value)), timeoutMs)
        ?: error("Timed out waiting for resource \"$value\"")
}
