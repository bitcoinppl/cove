package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupDeleteCategory
import org.bitcoinppl.cove_core.CloudBackupDeleteConfirmation
import org.bitcoinppl.cove_core.CloudBackupLastOutcome
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOperationKind
import org.bitcoinppl.cove_core.CloudBackupOutcome
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRejection
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.cloudBackupScreenFixtures
import org.bitcoinppl.cove_core.cloudBackupUiElements
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Open dialogs and prompts follow the screen Rust sends now
 *
 * A row's sheet and delete confirmation, and each root prompt, stay open while Rust changes the
 * entries under them; their controls must follow the new entries, never the ones they opened on
 */
@RunWith(AndroidJUnit4::class)
class CloudBackupDialogContractTest {
    @get:Rule
    val compose = createComposeRule()

    private val dispatched = mutableListOf<CloudBackupManagerAction>()
    private var detail by mutableStateOf<CloudBackupScreen?>(null)

    @Test
    fun everyTagMatchesTheIdentifierRustOwns() {
        val elements = cloudBackupUiElements()
        val mismatches =
            elements.mapNotNull { element ->
                val rust = element.accessibilityIdentifier()
                val kotlin = CloudBackupTags.of(element)
                "$element: Kotlin $kotlin, Rust $rust".takeIf { kotlin != rust }
            }

        assertTrue("expected Rust to list the tagged elements", elements.isNotEmpty())
        assertEquals("tags that differ from Rust's identifiers", emptyList<String>(), mismatches)
    }

    @Test
    fun anOpenRowSheetFollowsTheAvailabilityRustSendsNow() {
        val (start, row) = rowWithDelete()
        showDetail(start)

        compose.onNodeWithTag(CloudBackupTags.walletRow(row.recordId)).performScrollTo().performClick()
        val deleteTag = CloudBackupTags.row(row.recordId, row.delete().action)
        compose.onNodeWithTag(deleteTag).assertIsEnabled()

        show(start.withRow(row.recordId) { it.withDelete(availability = BUSY) })
        compose.onNodeWithTag(deleteTag).assertIsNotEnabled()
        compose.onNodeWithTag(deleteTag).performClick()
        compose.onAllNodesWithTag(CloudBackupTags.rowConfirm(row.recordId, row.delete().action)).assertCountEquals(0)

        // Rust stops listing the row, so the sheet bound to it closes
        show(start.withRow(row.recordId) { null })
        compose.onAllNodesWithTag(deleteTag).assertCountEquals(0)
        assertEquals("dispatched from a sheet Rust withdrew", emptyList<CloudBackupManagerAction>(), dispatched)
    }

    @Test
    fun anOpenDeleteConfirmationFollowsTheWarningRustSendsNow() {
        val (start, row) = rowWithDelete()
        val withDelete = { confirmation: CloudBackupDeleteConfirmation, availability: CloudBackupActionAvailability ->
            start.withRow(row.recordId) { it.withDelete(confirmation, availability) }
        }
        showDetail(withDelete(CloudBackupDeleteConfirmation.STANDARD, CloudBackupActionAvailability.Available))

        compose.onNodeWithTag(CloudBackupTags.walletRow(row.recordId)).performScrollTo().performClick()
        compose.onNodeWithTag(CloudBackupTags.row(row.recordId, row.delete().action)).performClick()
        compose.onNodeWithText(cloudBackupRowDeleteMessage(CloudBackupDeleteConfirmation.STANDARD)).assertIsDisplayed()

        show(withDelete(CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY, CloudBackupActionAvailability.Available))
        compose
            .onNodeWithText(cloudBackupRowDeleteMessage(CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY))
            .assertIsDisplayed()
        compose
            .onAllNodesWithText(cloudBackupRowDeleteMessage(CloudBackupDeleteConfirmation.STANDARD))
            .assertCountEquals(0)

        val confirmTag = CloudBackupTags.rowConfirm(row.recordId, row.delete().action)
        show(withDelete(CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY, BUSY))
        compose.onNodeWithTag(confirmTag).assertIsNotEnabled()
        compose.onNodeWithTag(confirmTag).performClick()

        // Rust stops offering the delete, so its confirmation closes
        show(
            start.withRow(row.recordId) { listed ->
                listed.copy(actions = listed.actions.filter { it.action.control != CloudBackupRowControl.DELETE })
            },
        )
        compose.onAllNodesWithTag(confirmTag).assertCountEquals(0)
        assertEquals("deleted after Rust withdrew the delete", emptyList<CloudBackupManagerAction>(), dispatched)
    }

    @Test
    fun anOpenDeleteConfirmationFollowsTheRowAcrossSections() {
        showDetail(cloudBackupFixture("configured"))

        val confirmTag = CloudBackupTags.rowConfirm(SPENDING_OLD, SPENDING_OLD_DELETE)
        compose.onNodeWithTag(CloudBackupTags.walletRow(SPENDING_OLD)).performScrollTo().performClick()
        compose.waitForIdle()
        compose.onNodeWithTag(CloudBackupTags.row(SPENDING_OLD, SPENDING_OLD_DELETE)).performClick()
        compose.waitForIdle()
        compose.onNodeWithTag(confirmTag).assertIsEnabled()

        // an operation starts elsewhere, so the open confirm can no longer run
        show(cloudBackupFixture("configured-rows-held"))
        compose.onNodeWithTag(confirmTag).assertIsNotEnabled()

        // the wallet left this device, so Rust lists its copy under Not on This Device; the open
        // confirmation stays and warns that the copy may be the only one
        show(cloudBackupFixture("configured-extra-copy-only-copy"))
        compose.onNodeWithText(cloudBackupRowDeleteMessage(CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY)).assertIsDisplayed()
        compose.onNodeWithTag(confirmTag).assertIsEnabled()

        // the copy was deleted elsewhere, so nothing is left to confirm
        show(cloudBackupFixture("configured-extra-copy-gone"))
        compose.onAllNodesWithTag(confirmTag).assertCountEquals(0)
        assertEquals("deleted from a test", emptyList<CloudBackupManagerAction>(), dispatched)
    }

    /**
     * Delete sends back the consent of the delete its dialog shows: the one a dialog replaced for
     * a stronger warning carries the consent Rust offered with it, never the one it opened with
     */
    @Test
    fun aDeleteConfirmationSendsBackTheConsentOfTheDeleteItShows() {
        showDetail(cloudBackupFixture("configured"))

        val confirmTag = CloudBackupTags.rowConfirm(SPENDING_OLD, SPENDING_OLD_DELETE)
        compose.onNodeWithTag(CloudBackupTags.walletRow(SPENDING_OLD)).performScrollTo().performClick()
        compose.waitForIdle()
        compose.onNodeWithTag(CloudBackupTags.row(SPENDING_OLD, SPENDING_OLD_DELETE)).performClick()
        compose.waitForIdle()

        val onlyCopy = cloudBackupFixture("configured-extra-copy-only-copy")
        show(onlyCopy)
        compose.onNodeWithTag(confirmTag).performClick()
        compose.waitForIdle()

        val offered = onlyCopy.listedRow(SPENDING_OLD)!!.delete().action as CloudBackupRowActionKind.Delete
        assertEquals(listOf(CloudBackupManagerAction.DeleteCloudWallet(offered.consent)), dispatched)
    }

    @Test
    fun anOpenPromptFollowsTheEntriesRustSendsNow() {
        val prompts =
            cloudBackupScreenFixtures().map { it.screen }.filter { screen ->
                val prompt = screen.rootPrompt
                prompt !is CloudBackupRootPrompt.None && prompt !is CloudBackupRootPrompt.MissingPasskeyReminder
            }
        assertTrue("expected fixtures for each prompt with choices", prompts.size >= 3)

        var shown by mutableStateOf(prompts.first())
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                shown.rootPrompt.toRootPresentation()?.let { presentation ->
                    CloudBackupRootPromptDialog(presentation, shown, promptHost())
                }
            }
        }

        val failures =
            prompts.flatMap { prompt ->
                val choices = prompt.promptChoiceActions()
                choices.flatMap { action ->
                    val tag = CloudBackupTags.action(action)
                    buildList {
                        shown = prompt.withEntry(action, CloudBackupActionAvailability.Available)
                        compose.waitForIdle()
                        if (enabledCount(tag) == 0) add("$action has no enabled choice")

                        // a choice dispatches and leaves the prompt to Rust
                        dispatched.clear()
                        compose.onAllNodesWithTag(tag)[0].performClick()
                        compose.waitForIdle()
                        if (dispatched.isEmpty()) add("$action dispatched nothing")
                        if (compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isEmpty()) {
                            add("$action closed the prompt before Rust answered")
                        }

                        shown = prompt.withEntry(action, BUSY).withRejection(action)
                        compose.waitForIdle()
                        if (enabledCount(tag) > 0) add("$action stayed enabled after Rust held it back")
                        if (compose.onAllNodesWithText(BUSY.caption!!).fetchSemanticsNodes().isEmpty()) {
                            add("$action doesn't say why it can't run")
                        }
                        if (compose.onAllNodesWithTag(CloudBackupTags.LAST_OUTCOME).fetchSemanticsNodes().isEmpty()) {
                            add("$action doesn't draw its rejection in the prompt")
                        }

                        shown = prompt.withEntry(action, null)
                        compose.waitForIdle()
                        if (compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty()) {
                            add("$action is drawn after Rust stopped sending it")
                        }
                    }.map { "${prompt.rootPrompt::class.simpleName}: $it" }
                }
            }

        assertEquals("prompt choices that don't follow Rust", emptyList<String>(), failures)
    }

    // the listed-row sections and the row dialog host as the detail screen draws them, from the
    // screen the test swaps; the sections scroll as the detail screen's frame does, so a row is
    // scrolled to before it is tapped
    private fun showDetail(start: CloudBackupScreen) {
        detail = start
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                val current = detail ?: return@CoveTheme
                val manager = remember { CloudBackupManager(current) }
                val rowDialogs = rememberCloudBackupRowDialogs()

                Column(Modifier.verticalScroll(rememberScrollState())) {
                    current.configured?.sections.orEmpty().forEach { section ->
                        when (section) {
                            is CloudBackupSection.NotOnDevice ->
                                NotOnDeviceSection(manager, current, section.v1, rowDialogs)
                            is CloudBackupSection.ExtraCopies ->
                                ExtraCopiesSection(manager, current, section.v1, rowDialogs)
                            else -> Unit
                        }
                    }
                }

                CloudBackupRowDialogHost(screen = current, dialogs = rowDialogs, onDispatch = { dispatched += it })
            }
        }

        compose.waitForIdle()
    }

    private fun show(next: CloudBackupScreen) {
        detail = next
        compose.waitForIdle()
    }

    private fun promptHost(): CloudBackupRootPromptHost =
        CloudBackupRootPromptHost(
            onDispatch = { dispatched += it },
            onClose = {},
            consumeHostDismiss = { false },
            onOpenCloudBackup = {},
        )

    private fun enabledCount(tag: String): Int =
        compose.onAllNodesWithTag(tag).fetchSemanticsNodes().count { node ->
            node.config.getOrNull(SemanticsProperties.Disabled) == null
        }

    // the first fixture row whose delete Rust offers, with the screen it is listed on
    private fun rowWithDelete(): Pair<CloudBackupScreen, CloudBackupListedRow> =
        cloudBackupScreenFixtures().firstNotNullOf { fixture ->
            fixture.screen.configured?.sections.orEmpty().firstNotNullOfOrNull { section ->
                val rows =
                    when (section) {
                        is CloudBackupSection.NotOnDevice -> section.v1.rows
                        is CloudBackupSection.ExtraCopies -> section.v1.rows
                        else -> emptyList()
                    }

                rows.firstOrNull { it.action(CloudBackupRowControl.DELETE)?.availability?.isAvailable == true }
                    ?.let { fixture.screen to it }
            }
        }

    private fun CloudBackupListedRow.delete() = action(CloudBackupRowControl.DELETE)!!

    // the screen with the row replaced wherever Rust lists it, or removed when `transform` gives null
    private fun CloudBackupScreen.withRow(
        recordId: String,
        transform: (CloudBackupListedRow) -> CloudBackupListedRow?,
    ): CloudBackupScreen {
        val configured = configured ?: error("fixture is not configured")
        val rows = { rows: List<CloudBackupListedRow> ->
            rows.mapNotNull { if (it.recordId == recordId) transform(it) else it }
        }
        val sections =
            configured.sections.map { section ->
                when (section) {
                    is CloudBackupSection.NotOnDevice ->
                        CloudBackupSection.NotOnDevice(section.v1.copy(rows = rows(section.v1.rows)))
                    is CloudBackupSection.ExtraCopies ->
                        CloudBackupSection.ExtraCopies(section.v1.copy(rows = rows(section.v1.rows)))
                    else -> section
                }
            }

        return copy(body = CloudBackupScreenBody.Configured(configured.copy(sections = sections)))
    }

    private fun CloudBackupListedRow.withDelete(
        confirmation: CloudBackupDeleteConfirmation = (delete().action as CloudBackupRowActionKind.Delete).confirmation,
        availability: CloudBackupActionAvailability = CloudBackupActionAvailability.Available,
    ): CloudBackupListedRow {
        val offered = delete().action as CloudBackupRowActionKind.Delete

        // Rust asks for another confirmation only with a delete of another category, so it
        // offers a new consent with it
        val category =
            when (confirmation) {
                CloudBackupDeleteConfirmation.STANDARD -> CloudBackupDeleteCategory.ExtraCopy
                CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY -> CloudBackupDeleteCategory.NotOnDevice
            }
        val consent =
            if (confirmation == offered.confirmation) {
                offered.consent
            } else {
                offered.consent.copy(evidence = offered.consent.evidence.copy(category = category))
            }

        return copy(
            actions =
                actions.map { rowAction ->
                    if (rowAction.action.control != CloudBackupRowControl.DELETE) return@map rowAction

                    rowAction.copy(action = CloudBackupRowActionKind.Delete(confirmation, consent), availability = availability)
                },
        )
    }

    // the actions a prompt draws as choices: the entries Rust sends for this prompt
    private fun CloudBackupScreen.promptChoiceActions(): List<CloudBackupScreenAction> =
        when (rootPrompt) {
            is CloudBackupRootPrompt.Verification -> listOf(CloudBackupScreenAction.VERIFY)
            else -> PROMPT_CHOICES.filter { actions.lookup(it) != null }
        }

    // the screen with the action's entry replaced, or removed for a null availability
    private fun CloudBackupScreen.withEntry(
        action: CloudBackupScreenAction,
        availability: CloudBackupActionAvailability?,
    ): CloudBackupScreen {
        val others = actions.entries.filter { it.action != action }
        val entry = availability?.let { CloudBackupActionEntry(action, it) }
        return copy(actions = CloudBackupActions(others + listOfNotNull(entry)), lastOutcome = null)
    }

    private fun CloudBackupScreen.withRejection(action: CloudBackupScreenAction): CloudBackupScreen =
        copy(
            lastOutcome =
                CloudBackupLastOutcome(
                    target = CloudBackupOutcomeTarget.Action(action),
                    outcome = CloudBackupOutcome.Rejected(BUSY_REASON),
                ),
        )

    private companion object {
        const val SPENDING_OLD = "record-spending-old"

        // tags name a row control, not what its delete carries
        val SPENDING_OLD_DELETE: CloudBackupRowActionKind
            get() = cloudBackupFixture("configured").listedRow(SPENDING_OLD)!!.action(CloudBackupRowControl.DELETE)!!.action

        val BUSY_REASON: CloudBackupRejection = CloudBackupRejection.OperationRunning(CloudBackupOperationKind.VERIFY)
        val BUSY: CloudBackupActionAvailability = CloudBackupActionAvailability.Unavailable(BUSY_REASON)

        val PROMPT_CHOICES =
            listOf(
                CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING,
                CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_CREATE_NEW,
                CloudBackupScreenAction.REPAIR_PASSKEY,
            )
    }
}
