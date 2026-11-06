package org.bitcoinppl.cove.flows.cloudbackup

import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getAllSemanticsNodes
import androidx.compose.ui.semantics.getOrNull
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Restore All against the scripted fixture cloud, with the batch held inside each wallet's
 * download so the tests can read its progress, see which wallet it reads, leave the screen and
 * cancel it between wallets
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupRestoreAllFullLaunchTest {
    private companion object {
        const val FIXTURE_WALLET_ONE = "Fixture Wallet One"
        const val CLOUD_BACKUP_ROW = "Cloud Backup"
        const val WALLET_ROW_TAG_PREFIX = "cloudBackup.wallet."
        const val NOT_ON_DEVICE = "Not on device"
        const val ALREADY_ON_DEVICE = "Already on this device"

        // badges of a wallet on this phone; Failed and Unknown are not proof that a restore landed
        val ON_DEVICE_BADGES = setOf("Confirmed", "Syncing", "Dirty")
        val STATUS_BADGES = ON_DEVICE_BADGES + setOf("Failed", NOT_ON_DEVICE, ALREADY_ON_DEVICE, "Unsupported", "Unknown")

        // the batch's authoritative fetch reads each listed record once before it restores any
        const val PREPARATION_DOWNLOADS = 1

        // the batch reads with consent allowed, while the inventory refetch a restored wallet can
        // start reads silently, so holds on this policy only catch the batch's own reads
        val BATCH_POLICY = CloudAccessPolicy.CONSENT_ALLOWED

        // every fixture wallet a batch can restore, after Fixture Wallet One came back in onboarding
        val CLOUD_ONLY_RECORD_IDS =
            setOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                ScriptedCloudBackupFixture.XPRV_WALLET_RECORD_ID,
            )

        val WALLET_NAMES =
            mapOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID to "Fixture Wallet Two",
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID to "Fixture Wallet Three",
                ScriptedCloudBackupFixture.XPRV_WALLET_RECORD_ID to ScriptedCloudBackupFixture.XPRV_WALLET_NAME,
            )
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun releaseHeldDownloads() {
        ScriptedCloudStorageAccess.releaseAllWalletDownloadHolds()
    }

    /**
     * Restore All with three cloud-only wallets and a cloud copy of the wallet already on the
     * phone: the progress names each wallet in turn with its completed/total count, the batch reads
     * one wallet at a time in the listed order, and nothing ends up on the phone twice
     */
    @Test
    fun runningBatchRestoresEachWalletInTurnAndSkipsTheCopyAlreadyOnDevice() {
        val duplicate = ScriptedCloudBackupFixture.DUPLICATE_WALLET_RECORD_ID
        val order = openDetailWithCloudOnlyWallets(listDuplicate = true)

        assertTrue(
            "Restore All must leave the copy out of its count",
            device.findObject(By.text("Restore All (4)")) == null,
        )

        order.forEach { recordId ->
            ScriptedCloudStorageAccess.holdWalletDownload(recordId, PREPARATION_DOWNLOADS, BATCH_POLICY)
        }
        device.clickText("Restore All (3)")

        order.forEachIndexed { index, recordId ->
            val name = WALLET_NAMES.getValue(recordId)
            assertTrue(
                "the batch should read $name after $index finished wallets; " +
                    "held=${ScriptedCloudStorageAccess.heldWalletDownloadRecordIds()}",
                ScriptedCloudStorageAccess.awaitWalletDownloadHold(recordId, 30_000L),
            )

            assertEquals(
                "the batch must restore one wallet at a time, in the listed order",
                setOf(recordId),
                ScriptedCloudStorageAccess.heldWalletDownloadRecordIds(),
            )
            waitForProgress(completed = index, total = 3, detail = "Restoring $name")
            waitForScreen("rows before $name should be on the phone and the rest still in the cloud") {
                order.take(index).all { showsRow(it, ON_DEVICE_BADGES) } &&
                    order.drop(index).all { showsRow(it, setOf(NOT_ON_DEVICE)) }
            }

            val local = localWalletNames()
            val finished = order.take(index).map(WALLET_NAMES::getValue)
            val pending = order.drop(index).map(WALLET_NAMES::getValue)
            assertTrue("wallets before $name should be on the phone, saw $local", local.containsAll(finished))
            assertTrue(
                "$name and the wallets after it should not be on the phone yet, saw $local",
                pending.none(local::contains),
            )

            ScriptedCloudStorageAccess.releaseWalletDownloadHold(recordId)
        }

        waitUntil("Restore All should finish with every cloud-only wallet on the phone") {
            CloudBackupManager.getInstance().restoreAllState is CloudBackupRestoreAllState.NotShown &&
                isCloudOnlyLoaded() &&
                restorableRecordIds().isEmpty() &&
                cloudOnlyWallet(duplicate)?.syncStatus == CloudBackupWalletStatus.ALREADY_ON_DEVICE &&
                localWalletNames().containsAll(WALLET_NAMES.values)
        }

        val local = AppManager.getInstance().wallets.map { wallet -> wallet.name }
        assertEquals(
            "each wallet should be on the phone exactly once",
            (WALLET_NAMES.values + FIXTURE_WALLET_ONE).sorted(),
            local.sorted(),
        )

        waitForScreen("the finished batch should show every wallet on the phone and the copy beside them") {
            val texts = detailTexts()

            progressCardTexts().isEmpty() &&
                (order + ScriptedCloudBackupFixture.WALLET_RECORD_ID).all { showsRow(it, ON_DEVICE_BADGES) } &&
                showsRow(duplicate, setOf(ALREADY_ON_DEVICE)) &&
                texts.none { it.startsWith("Restore All") || it.startsWith("Retry Remaining") }
        }
    }

    /**
     * Leaving Cloud Backup and Settings while Restore All runs keeps the batch going, coming back
     * shows its progress, and Cancel lets the current wallet finish and leaves the rest cloud-only
     */
    @Test
    fun leavingAndReenteringKeepsTheBatchAndCancelSkipsTheRemainingWallets() {
        val order = openDetailWithCloudOnlyWallets(listDuplicate = false)
        val (first, second, third) = order
        val names = order.map(WALLET_NAMES::getValue)

        ScriptedCloudStorageAccess.holdWalletDownload(first, PREPARATION_DOWNLOADS, BATCH_POLICY)
        ScriptedCloudStorageAccess.holdWalletDownload(second, PREPARATION_DOWNLOADS, BATCH_POLICY)
        device.clickText("Restore All (3)")
        assertTrue(
            "the batch should start restoring ${names[0]}",
            ScriptedCloudStorageAccess.awaitWalletDownloadHold(first, 30_000L),
        )

        waitForProgress(completed = 0, total = 3, detail = "Restoring ${names[0]}")

        // back out of Cloud Backup and Settings to the wallet
        device.pressBack()
        waitUntil("Back should leave Cloud Backup for Settings") {
            AppManager.getInstance().currentRoute == Route.Settings(SettingsRoute.Main)
        }

        device.pressBack()
        waitUntil("Back should leave Settings for the wallet") {
            AppManager.getInstance().currentRoute is Route.SelectedWallet
        }

        // with no Cloud Backup screen open, the batch finishes the wallet and moves to the next
        ScriptedCloudStorageAccess.releaseWalletDownloadHold(first)
        assertTrue(
            "the batch should keep going while Cloud Backup is closed",
            ScriptedCloudStorageAccess.awaitWalletDownloadHold(second, 30_000L),
        )

        waitUntil("the running batch should report ${names[1]} next") {
            val state = CloudBackupManager.getInstance().restoreAllState
            state is CloudBackupRestoreAllState.Running &&
                state.completed == 1u &&
                state.total == 3u &&
                state.currentWalletName == names[1] &&
                !state.cancellationRequested
        }

        assertTrue("${names[0]} should be on the phone", localWalletNames().contains(names[0]))

        device.openSettings()
        device.openCloudBackupFromSettings()
        waitForProgress(completed = 1, total = 3, detail = "Restoring ${names[1]}")
        waitForScreen("coming back should show ${names[0]} on the phone and the rest still in the cloud") {
            showsRow(first, ON_DEVICE_BADGES) && showsRow(second, setOf(NOT_ON_DEVICE)) && showsRow(third, setOf(NOT_ON_DEVICE))
        }

        assertEquals(
            "coming back must show the same batch, not start another",
            setOf(second),
            ScriptedCloudStorageAccess.heldWalletDownloadRecordIds(),
        )

        device.clickText("Cancel")
        waitForProgress(completed = 1, total = 3, detail = "Finishing the current wallet before stopping")

        assertEquals(
            "Cancel must let the current wallet's restore keep running",
            setOf(second),
            ScriptedCloudStorageAccess.heldWalletDownloadRecordIds(),
        )
        ScriptedCloudStorageAccess.releaseWalletDownloadHold(second)

        waitUntil("Cancel should stop the batch after ${names[1]} with ${names[2]} left in the cloud") {
            val state = CloudBackupManager.getInstance().restoreAllState
            val local = localWalletNames()

            state is CloudBackupRestoreAllState.RetryAvailable &&
                state.walletCount == 1u &&
                restorableRecordIds() == listOf(third) &&
                local.containsAll(names.take(2)) &&
                !local.contains(names[2])
        }

        device.scrollUntilText("Retry Remaining (1)")

        waitForScreen("the cancelled batch should show ${names[2]} alone left in the cloud") {
            val texts = detailTexts()

            progressCardTexts().isEmpty() &&
                showsRow(first, ON_DEVICE_BADGES) &&
                showsRow(second, ON_DEVICE_BADGES) &&
                showsRow(third, setOf(NOT_ON_DEVICE)) &&
                texts.none { it.startsWith("Restore All") }
        }
    }

    /**
     * Restores Fixture Wallet One in onboarding, then lists wallets two and three and the private
     * key wallet as well, and the copy of wallet one when [listDuplicate], so the detail offers
     * Restore All (3)
     *
     * Returns the Not on device rows' record ids in the order the screen shows them, which the
     * tests expect Restore All to follow
     */
    private fun openDetailWithCloudOnlyWallets(listDuplicate: Boolean): List<String> {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickText("Restore with Passkey")
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
        device.clickText("Done")
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        // onboarding's last step selects the restored wallet and resets the route, which would
        // replace a settings route opened before it
        val app = AppManager.getInstance()
        val finished =
            pollUntil(30_000L) {
                app.router.default is Route.SelectedWallet && app.router.routes.isEmpty()
            }
        assertTrue(
            "onboarding should finish on a selected wallet; default=${app.router.default}, " +
                "routes=${app.router.routes}, wallets=${app.wallets.map { it.name }}",
            finished,
        )

        val duplicate = listOf(ScriptedCloudBackupFixture.DUPLICATE_WALLET_RECORD_ID).takeIf { listDuplicate }.orEmpty()
        ScriptedCloudStorageAccess.exposeProductionFixtureWallets(
            listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID) + CLOUD_ONLY_RECORD_IDS + duplicate,
        )

        device.openSettings()
        val walletListsBeforeEntry = ScriptedCloudStorageAccess.walletListCount()
        device.openCloudBackupFromSettings()

        // the detail held from the restore can already look complete, so wait for the refresh the
        // entry starts, its cloud-only reads and the restore's upload confirmation to settle
        waitUntil("expected Restore All (3) once the entry refresh settles", timeoutMs = 60_000L) {
            val manager = CloudBackupManager.getInstance()
            val restoreAll = manager.restoreAllState

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeEntry &&
                manager.isDetailInventoryComplete &&
                !manager.isDetailInventoryChecking &&
                !manager.hasPendingUploadVerification &&
                restoreAll is CloudBackupRestoreAllState.StartAvailable &&
                restoreAll.walletCount == 3u &&
                restorableRecordIds().toSet() == CLOUD_ONLY_RECORD_IDS &&
                (!listDuplicate || cloudOnlyWallet(duplicate.single()) != null)
        }

        device.scrollUntilText("Restore All (3)")
        waitForScreen("the detail should show the three wallets as not on device and the copy as already on it") {
            CLOUD_ONLY_RECORD_IDS.all { showsRow(it, setOf(NOT_ON_DEVICE)) } &&
                duplicate.all { showsRow(it, setOf(ALREADY_ON_DEVICE)) }
        }

        val shown = walletRows().filter { row -> row.badge == NOT_ON_DEVICE }.map { row -> row.recordId }
        assertEquals("the screen should list the wallets in the order Restore All takes them", restorableRecordIds(), shown)

        return shown
    }

    // waits until the progress card on screen reads [completed] of [total] and [detail]; the card
    // merges its texts into one node, read from compose's semantics tree since uiautomator's view
    // of a merged node doesn't list them
    private fun waitForProgress(
        completed: Int,
        total: Int,
        detail: String,
    ) {
        val expected = listOf("$completed of $total complete", detail)
        var seen = emptyList<List<String>>()
        val shown =
            pollUntil(20_000L) {
                seen = progressCardTexts()
                seen.any { texts -> texts.containsAll(expected) }
            }

        assertTrue("the progress card should show $expected, saw $seen", shown)
    }

    private fun progressCardTexts(): List<List<String>> =
        mergedSemanticsNodes()
            .filter { node -> node.config.getOrNull(SemanticsProperties.ProgressBarRangeInfo) != null }
            .map { node -> node.texts() }
            .filter { texts -> texts.any { it.endsWith(" complete") } }

    // every text the detail composes, on screen or scrolled off, since its column composes all of it
    private fun detailTexts(): List<String> = mergedSemanticsNodes().flatMap { node -> node.texts() }

    private fun mergedSemanticsNodes(): List<SemanticsNode> {
        val nodes = mutableListOf<SemanticsNode>()
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            nodes +=
                WindowInspector
                    .getGlobalWindowViews()
                    .flatMap { it.composeRoots() }
                    .flatMap { root -> root.semanticsOwner.getAllSemanticsNodes(mergingEnabled = true) }
        }

        return nodes
    }

    private fun SemanticsNode.texts(): List<String> =
        config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text }

    private fun View.composeRoots(): List<RootForTest> =
        when (this) {
            is RootForTest -> listOf(this)
            is ViewGroup -> (0 until childCount).flatMap { getChildAt(it).composeRoots() }
            else -> emptyList()
        }

    private data class WalletRow(
        val recordId: String,
        val top: Float,
        val badge: String?,
    )

    // every wallet row the detail composes, top to bottom, named by the record in its test tag;
    // sorted by unclipped position, since clipped bounds put rows scrolled off screen at zero
    private fun walletRows(): List<WalletRow> =
        mergedSemanticsNodes()
            .mapNotNull { node ->
                val tag = node.config.getOrNull(SemanticsProperties.TestTag)
                val recordId = tag?.takeIf { it.startsWith(WALLET_ROW_TAG_PREFIX) }?.removePrefix(WALLET_ROW_TAG_PREFIX)

                recordId?.let {
                    WalletRow(it, node.positionInRoot.y, node.texts().firstOrNull(STATUS_BADGES::contains))
                }
            }.sortedBy { row -> row.top }

    // whether the screen shows exactly one row for the record, with one of [badges]
    private fun showsRow(
        recordId: String,
        badges: Set<String>,
    ): Boolean = walletRows().singleOrNull { row -> row.recordId == recordId }?.badge in badges

    // polls the composed detail, since it can trail the manager state by a frame or more
    private fun waitForScreen(
        message: String,
        timeoutMs: Long = 20_000L,
        condition: () -> Boolean,
    ) {
        if (pollUntil(timeoutMs, condition)) return

        val rows = walletRows().map { row -> row.recordId.take(8) to row.badge }
        assertTrue("$message; rows=$rows, progress=${progressCardTexts()}, texts=${detailTexts()}", condition())
    }

    private fun isCloudOnlyLoaded(): Boolean = CloudBackupManager.getInstance().cloudOnly is CloudOnlyState.Loaded

    private fun cloudOnlyWallets(): List<CloudBackupWalletItem> =
        (CloudBackupManager.getInstance().cloudOnly as? CloudOnlyState.Loaded)?.wallets.orEmpty()

    private fun cloudOnlyWallet(recordId: String): CloudBackupWalletItem? =
        cloudOnlyWallets().singleOrNull { wallet -> wallet.recordId == recordId }

    // cloud-only rows Restore All would take, in the detail's order
    private fun restorableRecordIds(): List<String> =
        cloudOnlyWallets()
            .filter { wallet -> wallet.syncStatus == CloudBackupWalletStatus.DELETED_FROM_DEVICE }
            .map { wallet -> wallet.recordId }

    private fun localWalletNames(): Set<String> =
        AppManager.getInstance().wallets.mapTo(mutableSetOf()) { wallet -> wallet.name }

    // pushed over the selected wallet as the wallet's settings button does, so Back returns to it;
    // resetting the route would make Settings the root, with nothing for Back to return to
    private fun UiDevice.openSettings() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().pushRoute(Route.Settings(SettingsRoute.Main))
        }

        waitForText("Security")
    }

    // taps the Settings row, so Back returns to Settings as it does for a person
    private fun UiDevice.openCloudBackupFromSettings() {
        clickText(CLOUD_BACKUP_ROW)
        waitUntil("the Cloud Backup row should open Cloud Backup") {
            AppManager.getInstance().currentRoute == Route.Settings(SettingsRoute.CloudBackup)
        }
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    // scrolls down, then back up, since the detail may already be scrolled past the text
    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var swipes = 0

        while (System.currentTimeMillis() < deadline) {
            findObject(By.text(value))?.let { return it }
            val upward = (swipes / 4) % 2 == 1
            val startY = if (upward) displayHeight / 4 else displayHeight * 3 / 4
            val endY = if (upward) displayHeight * 3 / 4 else displayHeight / 4
            swipe(displayWidth / 2, startY, displayWidth / 2, endY, 20)
            swipes += 1
            Thread.sleep(250)
        }

        error("Timed out scrolling to text \"$value\"")
    }

    // clicks the first node showing [value] that sits in something clickable, since a text can also
    // appear as a plain label, such as the Cloud Backup section header above its Settings row
    private fun UiDevice.clickText(value: String) {
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            try {
                scrollUntilText(value)
                val clickable =
                    findObjects(By.text(value)).firstNotNullOfOrNull { node ->
                        generateSequence(node) { it.parent }.firstOrNull { it.isClickable }
                    }

                if (clickable != null) {
                    clickable.click()
                    return
                }
            } catch (_: StaleObjectException) {
                // the tree moved under the read, so find the text again
            }

            Thread.sleep(100)
        }

        error("Timed out clicking text \"$value\"")
    }

    private fun pollUntil(
        timeoutMs: Long,
        condition: () -> Boolean,
    ): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return true

            Thread.sleep(100)
        }

        return condition()
    }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 30_000L,
        condition: () -> Boolean,
    ) {
        if (pollUntil(timeoutMs, condition)) return

        val manager = CloudBackupManager.getInstance()
        val diagnostics =
            "restoreAll=${manager.restoreAllState}, cloudOnly=${cloudOnlyWallets().map { it.recordId to it.syncStatus }}, " +
                "route=${AppManager.getInstance().currentRoute}, localWallets=${localWalletNames()}, " +
                "held=${ScriptedCloudStorageAccess.heldWalletDownloadRecordIds()}, " +
                "walletDownloads=${ScriptedCloudStorageAccess.walletDownloadRecordIds()}"
        assertTrue("$message; $diagnostics", condition())
    }
}
