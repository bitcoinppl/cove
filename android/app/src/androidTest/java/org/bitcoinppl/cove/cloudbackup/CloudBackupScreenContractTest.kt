package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupControlSurface
import org.bitcoinppl.cove_core.CloudBackupDisableConfirmation
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupDrawnControl
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.cloudBackupScreenFixtures
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Every Rust screen fixture draws what its actions allow, on the surface Rust names for it
 *
 * Rust names each control and the surface that draws it (`drawnControls`). An available action
 * must have an enabled control on each of those surfaces: the overflow menu is opened only for a
 * control Rust puts in the menu, so a control the screen should draw in place can't pass by being
 * in the menu, and the next Turn Off step or the setup acknowledgements are opened where that is
 * where it lives. An unavailable one must be disabled or absent. The fixtures come from the Rust
 * library, so a projection change fails here before it reaches a person
 */
@RunWith(AndroidJUnit4::class)
class CloudBackupScreenContractTest {
    @get:Rule
    val compose = createComposeRule()

    private var shown by mutableStateOf<Shown?>(null)

    @Test
    fun fixturesCoverEveryScreenTheContractNames() {
        assertTrue(
            "expected the debug Rust library to return screen fixtures",
            cloudBackupScreenFixtures().isNotEmpty(),
        )
    }

    // needs Rust part B: the interim projection offers RefreshDetail on every configured screen
    // and DiscardPendingEnable on pending-enable recovery, which Android draws no control for; see
    // interimUndrawn
    @Test
    fun everyAvailableActionHasAnEnabledControlOnItsSurface() {
        setFrame()

        val failures =
            cloudBackupScreenFixtures().flatMap { fixture ->
                val screen = fixture.screen
                val drawn = screen.drawnControls().distinct()
                val unplaced =
                    screen.actions.entries
                        .filter { entry -> drawn.none { it.action == entry.action } }
                        .map { "${fixture.shortName}: ${it.action} has no surface" }

                unplaced +
                    drawn
                        .filter { screen.actions.lookup(it.action) is CloudBackupActionAvailability.Available }
                        .filterNot { (fixture.shortName to it.action) in interimUndrawn }
                        .mapNotNull { control ->
                            show(screen)
                            reach(screen, control)
                            enabledControlFailure(CloudBackupTags.action(control.action))
                                ?.let { "${fixture.shortName}: ${control.action} on ${control.surface} $it" }
                        }
            }

        assertEquals("available actions without an enabled control", emptyList<String>(), failures)
    }

    @Test
    fun everyUnavailableActionIsDisabledOrAbsent() {
        setFrame()

        val failures =
            cloudBackupScreenFixtures().flatMap { fixture ->
                val screen = fixture.screen
                screen.drawnControls()
                    .distinct()
                    .filter { screen.actions.lookup(it.action) is CloudBackupActionAvailability.Unavailable }
                    .mapNotNull { control ->
                        show(screen)
                        reach(screen, control)

                        val enabled = enabledNodes(CloudBackupTags.action(control.action))
                        "${fixture.shortName}: ${control.action} has an enabled control".takeIf { enabled > 0 }
                    }
            }

        assertEquals("unavailable actions with an enabled control", emptyList<String>(), failures)
    }

    @Test
    fun everyRowActionHasAControlInItsSheetMatchingItsAvailability() {
        setFrame()

        val failures =
            cloudBackupScreenFixtures().flatMap { fixture ->
                fixture.screen.listedRows().flatMap { row ->
                    row.actions.mapNotNull { rowAction ->
                        show(fixture.screen)
                        compose.onNodeWithTag(CloudBackupTags.rowOpen(row.recordId)).performScrollTo().performClick()
                        compose.waitForIdle()

                        val tag = CloudBackupTags.row(row.recordId, rowAction.action)
                        val failure =
                            when (rowAction.availability) {
                                CloudBackupActionAvailability.Available -> enabledControlFailure(tag)
                                is CloudBackupActionAvailability.Unavailable ->
                                    "has an enabled control".takeIf { enabledNodes(tag) > 0 }
                            }

                        failure?.let { "${fixture.shortName}: row ${row.recordId} ${rowAction.action} $it" }
                    }
                }
            }

        assertEquals("row actions whose control doesn't match", emptyList<String>(), failures)
    }

    private fun setFrame() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                shown?.let { shown ->
                    // a new key drops every dialog, sheet and menu the previous check opened
                    key(shown.round) {
                        val manager = remember { CloudBackupManager(shown.screen) }

                        CloudBackupScreenFrame(
                            manager = manager,
                            actions = CloudBackupScreenActions(onBack = {}, onRecreate = {}, onReinitialize = {}),
                        )
                    }
                }
            }
        }
    }

    private fun show(screen: CloudBackupScreen) {
        shown = Shown(round = (shown?.round ?: 0) + 1, screen = screen)
        compose.waitForIdle()
    }

    // opens whatever holds the control: the overflow menu when Rust puts it there, the final
    // Turn Off step, or the setup acknowledgements that gate Enable
    private fun reach(
        screen: CloudBackupScreen,
        control: CloudBackupDrawnControl,
    ) {
        if (control.surface == CloudBackupControlSurface.MENU) {
            compose.onNodeWithTag(CloudBackupTags.MENU).performClick()
            compose.waitForIdle()
            return
        }

        when (control.action) {
            CloudBackupScreenAction.CONFIRM_DISABLE -> {
                if (screen.reviewConfirmation() == CloudBackupDisableConfirmation.TWO) {
                    compose.onNodeWithTag(CloudBackupTags.TURN_OFF_REVIEW_CONTINUE).performClick()
                }
            }
            CloudBackupScreenAction.ENABLE,
            CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
            -> ACKNOWLEDGEMENTS.forEach { checkRow(it).performScrollTo().performClick() }
            else -> Unit
        }

        compose.waitForIdle()
    }

    private fun enabledControlFailure(tag: String): String? {
        val nodes = compose.onAllNodesWithTag(tag).fetchSemanticsNodes()
        if (nodes.isEmpty()) return "has no control tagged $tag"

        val enabled = enabledNodes(tag)
        if (enabled == 0) return "has only disabled controls tagged $tag"

        runCatching { compose.onAllNodesWithTag(tag)[0].performScrollTo() }
        return null
    }

    private fun enabledNodes(tag: String): Int =
        compose.onAllNodesWithTag(tag).fetchSemanticsNodes().count { node ->
            node.config.getOrNull(SemanticsProperties.Disabled) == null
        }

    private fun checkRow(text: String): SemanticsNodeInteraction =
        compose.onNode(
            hasClickAction() and hasAnyDescendant(hasText(text, substring = true)),
            useUnmergedTree = true,
        )

    private fun CloudBackupScreen.listedRows(): List<CloudBackupListedRow> =
        configured?.sections.orEmpty().flatMap { section ->
            when (section) {
                is CloudBackupSection.NotOnDevice -> section.v1.rows
                is CloudBackupSection.ExtraCopies -> section.v1.rows
                else -> emptyList()
            }
        }

    private fun CloudBackupScreen.reviewConfirmation(): CloudBackupDisableConfirmation? =
        (configured?.disable as? CloudBackupDisableScreen.Reviewing)?.v1?.impact?.confirmation

    private data class Shown(
        val round: Int,
        val screen: CloudBackupScreen,
    )

    private companion object {
        val ACKNOWLEDGEMENTS =
            listOf(
                "my passkey is required",
                "need access to my Google account",
                "manually back up my 12 or 24 words",
            )

        // actions the interim Rust projection offers without a section that draws them; Rust part
        // B's projection owns removing them or adding the section, and this list goes with it
        val interimUndrawn =
            setOf(
                "configured" to CloudBackupScreenAction.REFRESH_DETAIL,
                "configured-passkey-missing" to CloudBackupScreenAction.REFRESH_DETAIL,
                "configured-operation-running" to CloudBackupScreenAction.REFRESH_DETAIL,
                "configured-heal-notice" to CloudBackupScreenAction.REFRESH_DETAIL,
                "turn-off-review" to CloudBackupScreenAction.REFRESH_DETAIL,
                "pending-enable-recovery" to CloudBackupScreenAction.DISCARD_PENDING_ENABLE,
            )
    }
}
