package org.bitcoinppl.cove.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupPublishedScreen
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupScreenDelta
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The manager starts from a revisioned snapshot and skips the deltas queued before it
 *
 * A Turn Off that finished before the snapshot was read must not be replayed over it as a
 * FinishingDisable to Disabled transition
 */
@RunWith(AndroidJUnit4::class)
class CloudBackupManagerHandoffTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val bindingStore = SharedPreferencesDriveAccountBindingStore(context)
    private val boundAccount =
        DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")

    // the same release CoveApplication registers, on the same store
    @Before
    fun registerTheBindingRelease() {
        CloudBackupManager.setOnCloudBackupSettledOff { clearCloudBackupDriveAccountBinding(context) }
        bindingStore.clearIdentity()
    }

    @After
    fun clearTheTestBinding() {
        bindingStore.clearIdentity()
    }

    /**
     * A full wipe that starts while Turn Off drains shows Disabled before anything commits, and one
     * that fails before its key clear resumes the configured backup, so the Drive account binding
     * stays until Rust reports Cloud Backup settled off
     */
    @Test
    fun theDriveAccountBindingGoesOnlyWhenRustReportsCloudBackupSettledOff() {
        val configured = cloudBackupFixture("configured")
        val configuredBody = configured.body as CloudBackupScreenBody.Configured
        val draining =
            CloudBackupScreenBody.Configured(
                configuredBody.v1.copy(disable = CloudBackupDisableScreen.Draining),
            )
        val manager = CloudBackupManager(CloudBackupPublishedScreen(1UL, configured))
        bindingStore.bindIdentity(boundAccount)

        manager.apply(screenDelta(2UL, draining))
        manager.apply(screenDelta(3UL, CloudBackupScreenBody.Disabled))
        assertTrue(manager.screen.body is CloudBackupScreenBody.Disabled)
        assertEquals(boundAccount, bindingStore.selectedIdentity())

        manager.apply(screenDelta(4UL, configured.body))
        assertEquals(boundAccount, bindingStore.selectedIdentity())

        manager.apply(CloudBackupReconcileMessage.SettledOff)
        assertNull(bindingStore.selectedIdentity())
    }

    @Test
    fun screenDeltasQueuedBeforeTheSnapshotAreSkipped() {
        val configured = cloudBackupFixture("configured")
        val finishing = cloudBackupFixture("finishing-disable")
        val disabled = cloudBackupFixture("disabled")
        val snapshot = CloudBackupPublishedScreen(3UL, configured)
        val manager = CloudBackupManager(snapshot)

        manager.apply(screenDelta(1UL, finishing.body))
        assertEquals(snapshot, manager.published)

        manager.apply(screenDelta(2UL, disabled.body))
        assertEquals(snapshot, manager.published)

        // an event has no revision, so it arrives in order even among skipped deltas
        manager.apply(
            CloudBackupReconcileMessage.EnableCompleted(
                CloudBackupEnableContext(
                    savedPasskeyConfirmation = SavedPasskeyConfirmationMode.MANUAL,
                    verificationSource = CloudBackupVerificationSource.SETTINGS,
                ),
            ),
        )
        assertNotNull(manager.enableCompletion)

        manager.apply(screenDelta(3UL, configured.body))
        assertEquals(snapshot, manager.published)
        assertNotNull(manager.enableCompletion)

        manager.apply(screenDelta(4UL, disabled.body))
        assertEquals(4UL, manager.published.revision)
        assertTrue(manager.screen.body is CloudBackupScreenBody.Disabled)
        assertNull(manager.enableCompletion)
    }

    private fun screenDelta(
        revision: ULong,
        body: CloudBackupScreenBody,
    ): CloudBackupReconcileMessage =
        CloudBackupReconcileMessage.Screen(
            CloudBackupScreenDelta(
                revision = revision,
                body = body,
                settingsRow = null,
                healNotice = null,
                lastOutcome = null,
                rootPrompt = null,
                actions = null,
            ),
        )
}
