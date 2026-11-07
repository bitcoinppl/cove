package org.bitcoinppl.cove.flows

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.toSize
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.NewWalletFlow.NewWalletSelectScreen
import org.bitcoinppl.cove.flows.SelectedWalletFlow.WalletBalanceHeaderView
import org.bitcoinppl.cove.flows.settings.WalletSettingsXprvExportDialog
import org.bitcoinppl.cove.flows.settings.XprvExportDialog
import org.bitcoinppl.cove.flows.settings.XprvRevealDialog
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.SystemFontScaleRule
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.test.shrunkTextProblem
import org.bitcoinppl.cove.test.textLayoutProblem
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.wallet.ExportXpubDialog
import org.bitcoinppl.cove.wallet.ExportXpubQrView
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.RustWalletManager
import org.bitcoinppl.cove_core.WalletScanStatus
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * The private key export dialogs, the Add New Wallet choices, the selected wallet's Send and
 * Receive buttons and the xpub export draw every text whole at the largest system font size,
 * with no word broken across lines, and keep each action whole and apart from the others. The
 * wallet choices and Send and Receive autosize their labels, so those must also keep their full size
 *
 * Dialogs and sheets are their own windows, which ignore a Compose test font scale, so this sets
 * the system font size before the activity starts and restores it afterwards. Each screen is
 * composed on its own, and each check reads every text in every window, so a text the test
 * doesn't name is checked too
 */
@LayoutRegressionTest
@RunWith(AndroidJUnit4::class)
class LargeTextWordBreakLayoutTest {
    private val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(SystemFontScaleRule(LARGEST_SYSTEM_FONT_SCALE)).around(compose)

    private var importedWalletId: WalletId? = null
    private var xpubManager: WalletManager? = null

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @Test
    fun privateKeyExportDialogsFitAtTheLargestFontScale() {
        var step by mutableStateOf(XprvExportStep.WARNING)

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                when (step) {
                    XprvExportStep.WARNING ->
                        WalletSettingsXprvExportDialog(dialog = XprvExportDialog.Warning, onDismiss = {}, onContinue = {}, onExport = {})

                    XprvExportStep.OPTIONS ->
                        WalletSettingsXprvExportDialog(dialog = XprvExportDialog.Options, onDismiss = {}, onContinue = {}, onExport = {})

                    XprvExportStep.REVEAL -> XprvRevealDialog(xprv = TEST_XPRV, onDismiss = {})
                }
            }
        }

        assertActionsFit("xprv warning", "Continue", "Cancel")
        assertEveryTextDrawsWhole("xprv warning")

        step = XprvExportStep.OPTIONS
        assertActionsFit("xprv export options", "Reveal", "KeyTeleport", "Cancel")
        assertEveryTextDrawsWhole("xprv export options")

        // the 111-character key has to wrap, so the check only fails a word that would fit a line
        step = XprvExportStep.REVEAL
        assertActionsFit("revealed xprv", "Copy", "Done")
        assertEveryTextDrawsWhole("revealed xprv")
    }

    @Test
    fun addNewWalletChoicesFitAtTheLargestFontScale() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                NewWalletSelectScreen(
                    app = AppManager.getInstance(),
                    onBack = {},
                    canGoBack = true,
                    onOpenNewHotWallet = {},
                    onOpenQrScan = {},
                    onOpenNfcScan = {},
                )
            }
        }

        assertActionsFit("add new wallet", "Hardware Wallet", "On This Device")
        assertLabelsAtFullSize("add new wallet", "Hardware Wallet", "On This Device")
        assertEveryTextDrawsWhole("add new wallet")
    }

    @Test
    fun sendAndReceiveFitAtTheLargestFontScale() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WalletBalanceHeaderView(
                    sensitiveVisible = true,
                    primaryAmount = "1,166,369 SATS",
                    secondaryAmount = "$1,351.93",
                    balancePresentation = remember { RustWalletManager.previewNewWallet().balancePresentation(WalletScanStatus.Idle) },
                    onToggleUnit = {},
                    onToggleSensitive = {},
                    onSend = {},
                    onReceive = {},
                )
            }
        }

        assertActionsFit("selected wallet", "Send", "Receive")
        assertLabelsAtFullSize("selected wallet", "Send", "Receive")
        assertEveryTextDrawsWhole("selected wallet")
    }

    @Test
    fun xpubExportFitsAtTheLargestFontScale() {
        // a preview wallet has no stored keys, so its export fails and the sheet shows only an error;
        // the test owns this manager instead of the app's cache, which closes managers it doesn't
        // consider current
        val manager = runBlocking { WalletManager.load(xpubWalletId()) }.also { xpubManager = it }
        var showsQrCode by mutableStateOf(false)

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                if (showsQrCode) {
                    ExportXpubQrView(manager)
                } else {
                    ExportXpubDialog(onShowQrCode = {}, onShare = {}, onDismiss = {})
                }
            }
        }

        assertActionsFit("xpub export options", "QR Code", "Share...", "Cancel")
        assertEveryTextDrawsWhole("xpub export options")

        // the title also shows over a failed export, so only the QR image proves the export worked
        showsQrCode = true
        val exported =
            runCatching {
                compose.waitUntil(STATE_TIMEOUT_MILLIS) {
                    compose.onAllNodes(hasContentDescription(XPUB_QR_DESCRIPTION)).fetchSemanticsNodes().isNotEmpty()
                }
            }.isSuccess

        assertTrue("xpub QR export never showed its QR code; the sheet showed ${drawnTexts()}", exported)
        assertEveryTextDrawsWhole("xpub QR export")
    }

    /**
     * The wallet for the published test words, imported if this device has none, and deleted after
     * the test only when this test imported it
     */
    private fun xpubWalletId(): WalletId {
        importTestWalletOrNull(XPUB_TEST_WALLET)?.let { imported ->
            importedWalletId = imported.id
            return imported.id
        }

        return Database().wallets().all().first { it.masterFingerprint?.asLowercase() == XPUB_TEST_WALLET.masterFingerprint }.id
    }

    // the wallet can't be deleted while this test's manager for it is open
    @After
    fun closeAndDeleteXpubWallet() {
        xpubManager?.close()
        importedWalletId?.let(::deleteTestWallet)
    }

    private fun drawnTexts(): List<String> =
        compose
            .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .flatMap { node -> node.config[SemanticsProperties.Text].map { it.text } }

    // MARK: layout checks

    /**
     * Each action must show whole in its window once scrolled to, take a click, be enabled, and
     * lay its label out whole, and no two of the actions may overlap
     */
    private fun assertActionsFit(
        screen: String,
        vararg labels: String,
    ) {
        compose.waitForIdle()

        val bounds =
            labels.map { label ->
                val name = "$screen: \"$label\""
                val action = compose.onNode(hasText(label) and hasClickAction(), useUnmergedTree = false)
                assertShowsWhole(action, name)
                action.assertIsEnabled()
                assertTextFits(compose.onNode(hasText(label) and hasTextLayout(), useUnmergedTree = true), "$name label")

                val semantics = action.fetchSemanticsNode("$name is missing")
                label to Rect(semantics.positionInRoot, semantics.size.toSize())
            }

        bounds.forEachIndexed { index, (label, rect) ->
            bounds.drop(index + 1).forEach { (otherLabel, other) ->
                val shared = rect.intersect(other)
                assertTrue(
                    "$screen: \"$label\" at $rect overlaps \"$otherLabel\" at $other",
                    shared.width <= CLIP_TOLERANCE_PX || shared.height <= CLIP_TOLERANCE_PX,
                )
            }
        }
    }

    /**
     * Scrolls to the node when it sits in a scroll area, then fails unless it lies inside its
     * window's width and shows whole there, not cut off by a container or the window's edge
     *
     * `boundsInRoot` is clipped by every ancestor, while the node's position and size are not
     */
    private fun assertShowsWhole(
        node: SemanticsNodeInteraction,
        name: String,
    ) {
        if (node.fetchSemanticsNode("$name is missing").isInsideScrollArea()) node.performScrollTo()
        compose.waitForIdle()

        val semantics = node.fetchSemanticsNode("$name is missing")
        val whole = Rect(semantics.positionInRoot, semantics.size.toSize())
        val shown = semantics.boundsInRoot
        val width = generateSequence(semantics) { it.parent }.last().size.width

        assertTrue(
            "$name runs past the side of its window at $whole in a ${width}px wide window",
            whole.left >= -CLIP_TOLERANCE_PX && whole.right <= width + CLIP_TOLERANCE_PX,
        )
        assertTrue(
            "$name shows only $shown of $whole",
            whole.left >= shown.left - CLIP_TOLERANCE_PX &&
                whole.top >= shown.top - CLIP_TOLERANCE_PX &&
                whole.right <= shown.right + CLIP_TOLERANCE_PX &&
                whole.bottom <= shown.bottom + CLIP_TOLERANCE_PX,
        )
    }

    /** These labels autosize down to fit, so each must still draw at its full font size */
    private fun assertLabelsAtFullSize(
        screen: String,
        vararg labels: String,
    ) {
        val problems =
            labels.mapNotNull { label ->
                val semantics = compose.onNode(hasText(label) and hasTextLayout(), useUnmergedTree = true).fetchSemanticsNode("$screen: \"$label\" is missing")
                compose.runOnIdle { semantics.shrunkTextProblem() }?.let { "\"$label\": $it" }
            }

        assertTrue("$screen has labels drawn below their font size: $problems", problems.isEmpty())
    }

    private fun assertTextFits(
        node: SemanticsNodeInteraction,
        name: String,
    ) {
        val semantics = node.fetchSemanticsNode("$name is missing")
        val problem = compose.runOnIdle { semantics.textLayoutProblem() }
        assertTrue("$name doesn't draw whole: $problem", problem == null)
    }

    /**
     * Every text in every window, named or not, must lay out whole; text fields scroll their own
     * entry by design
     *
     * Texts are found by their text rather than their layout action, so one that stops reporting
     * its layout fails instead of dropping out of the check
     */
    private fun assertEveryTextDrawsWhole(screen: String) {
        compose.waitForIdle()

        val texts =
            compose
                .onAllNodes(
                    SemanticsMatcher.keyIsDefined(SemanticsProperties.Text) and
                        !SemanticsMatcher.keyIsDefined(SemanticsProperties.EditableText),
                    useUnmergedTree = true,
                ).fetchSemanticsNodes()

        assertTrue("$screen has no texts to check", texts.isNotEmpty())
        val problems =
            compose.runOnIdle {
                texts.mapNotNull { node ->
                    val text = node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString(" ") { it.text }
                    node.textLayoutProblem()?.let { "\"${text.take(TEXT_IN_MESSAGE)}\": $it" }
                }
            }

        assertTrue("$screen has texts that don't draw whole: $problems", problems.isEmpty())
    }

    private fun hasTextLayout(): SemanticsMatcher = SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult)

    // a node outside any scroll area has nothing to scroll it into view, and performScrollTo fails there
    private fun SemanticsNode.isInsideScrollArea(): Boolean = generateSequence(parent) { it.parent }.any { SemanticsActions.ScrollBy in it.config }

    private enum class XprvExportStep {
        WARNING,
        OPTIONS,
        REVEAL,
    }

    private companion object {
        const val STATE_TIMEOUT_MILLIS = 15_000L

        // positions in Compose are floats, and a box rounds its edges to whole pixels
        const val CLIP_TOLERANCE_PX = 1f

        // enough of a text to tell which one a failure names
        const val TEXT_IN_MESSAGE = 40

        // the master key of BIP32 test vector 1, never used for funds
        const val TEST_XPRV =
            "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"

        const val XPUB_QR_DESCRIPTION = "QR code for transaction signing"

        // the published "abandon … about" BIP39 test vector, never used for funds; the onboarding
        // tests import it too, so an existing wallet for it is reused and left in place
        val XPUB_TEST_WALLET =
            TestWalletWords(
                List(11) { "abandon" } + "about",
                masterFingerprint = "73c5da0a",
            )
    }
}
