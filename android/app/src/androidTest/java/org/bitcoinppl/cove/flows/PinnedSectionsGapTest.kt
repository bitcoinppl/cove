package org.bitcoinppl.cove.flows

import androidx.activity.ComponentActivity
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.toSize
import androidx.lifecycle.Lifecycle
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.PendingWalletManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletCreateScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletImportScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.VerificationCompleteScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.VerifyWordsScreen
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCloudBackupSuccessView
import org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails.TransactionDetailsScreen
import org.bitcoinppl.cove.flows.SendFlow.ConfirmScreen.SendFlowConfirmScreen
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.flows.SendFlow.SendState
import org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen.CoinControlSetAmountScreen
import org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen.SendFlowSetAmountScreen
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerAdvancedChainCode
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerChooseChainCode
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerImportRetryView
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerImportSaveState
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerImportSuccessContent
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerManager
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerSetupRetryView
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerSetupSuccessContent
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.CompactViewport
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveAndroidComposeRule
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.TransactionDetailsPresentation
import org.bitcoinppl.cove_core.WordValidator
import org.bitcoinppl.cove_core.WordVerifyStateMachine
import org.bitcoinppl.cove_core.tapSignerSetupRetryContinueCmd
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.tapcard.tapSignerPreviewNew
import org.bitcoinppl.cove_core.types.confirmDetailsPreviewNew
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

// matches the spacing the screens keep between their pinned sections, as iOS does with VStack spacing
private val MIN_SECTION_GAP = 24.dp

// layout rounds each position to a whole pixel, so a gap can land a fraction of a dp short
private val GAP_ROUNDING_TOLERANCE = 0.5.dp

private const val VIEWPORT_TAG = "pinnedSections.viewport"

/**
 * Every screen that spreads its sections over the screen height must keep them apart, and its
 * primary control reachable, once its content outgrows a compact viewport, at the default and
 * the largest font scale
 *
 * Spreading only the leftover space between the sections, as `Arrangement.SpaceBetween` or a
 * weighted spacer does, leaves no gap once nothing is left over, so the sections touch on a
 * small screen or at a large display or font size, as the Verify Recovery Words dots did against
 * the word chips. A column that can't scroll goes further: its content overflows the space it
 * was given and draws over its neighbours or past the screen edge. A check that each control
 * fits the viewport misses both. Add a screen with this layout to [PinnedSectionsScreen]
 */
@LayoutRegressionTest
@RunWith(Parameterized::class)
class PinnedSectionsGapTest(
    private val screen: PinnedSectionsScreen,
    private val fontScale: Float,
) {
    private val compose = coveAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(AndroidDeviceStayAwakeRule())
            .around(compose)

    private val closeables = mutableListOf<AutoCloseable>()

    // every broken expectation of the case, so one run shows each way a screen falls apart
    private val failures = mutableListOf<String>()

    // the density the viewport laid out at, which is lower than the activity's on an emulator too
    // short to fit it
    private lateinit var layoutDensity: Density

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @After
    fun closeFixtures() {
        closeables.forEach { it.close() }
        closeables.clear()
    }

    @Test
    fun pinnedSectionsStayApart() {
        val content = screen.setUp(closeables)

        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.setContent {
            CoveTheme(darkTheme = true) {
                CompactViewport(fontScale, VIEWPORT_TAG, onDensity = { layoutDensity = it }) {
                    content()
                }
            }
        }
        compose.waitForIdle()

        checkSectionsApart()
        checkOnScreen()

        assertTrue(
            "${screen.name} at ${fontScale}x font:\n" + failures.joinToString("\n"),
            failures.isEmpty(),
        )
    }

    private fun check(
        condition: Boolean,
        message: () -> String,
    ) {
        if (!condition) failures += message()
    }

    private fun checkSectionsApart() {
        val extents = screen.sectionTags.map { tag -> tag to sectionExtent(tag) }

        extents.zipWithNext().forEach { (upper, lower) ->
            val gap = lower.second.top - upper.second.bottom

            check(gap >= MIN_SECTION_GAP - GAP_ROUNDING_TOLERANCE) {
                "${lower.first} should start at least $MIN_SECTION_GAP below ${upper.first}, but the gap is $gap"
            }
        }
    }

    // scrolls the page to each of the screen's primary controls and checks it draws whole inside
    // the viewport, so a control the page can't reach, or one an inner list can't scroll fully
    // into view, fails
    private fun checkOnScreen() {
        val viewportBounds = compose.onNodeWithTag(VIEWPORT_TAG).fetchSemanticsNode().boundsInRoot

        screen.onScreenTags.forEach { tag ->
            scrollIntoView(tag)

            val node = compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().firstOrNull()
            if (node == null) {
                failures += "$tag should be reachable, but no list scrolls it into the composition"
                return@forEach
            }

            val whole = node.ownBoundsPx().toLayoutDp()
            val drawn = node.boundsInRoot.toLayoutDp()
            val bounds = viewportBounds.toLayoutDp()

            check(whole.top >= bounds.top - GAP_ROUNDING_TOLERANCE && whole.bottom <= bounds.bottom + GAP_ROUNDING_TOLERANCE) {
                "$tag should be drawn whole inside the viewport, but it spans ${whole.top} to ${whole.bottom} " +
                    "in a viewport of ${bounds.top} to ${bounds.bottom}"
            }
            check(whole.bottom - whole.top - (drawn.bottom - drawn.top) <= GAP_ROUNDING_TOLERANCE) {
                "$tag should not be clipped, but only ${drawn.top} to ${drawn.bottom} of ${whole.top} to ${whole.bottom} draws"
            }
        }
    }

    private fun scrollIntoView(tag: String) {
        scrollLazyListsTo(tag)

        val scrollParents =
            compose
                .onAllNodes(hasScrollAction() and hasAnyDescendant(hasTestTag(tag)), useUnmergedTree = true)
                .fetchSemanticsNodes()
        if (scrollParents.isEmpty()) return

        compose.onNodeWithTag(tag, useUnmergedTree = true).performScrollTo()
        compose.waitForIdle()
    }

    // a lazy list composes only the items near its scroll position, so when the tag isn't composed
    // yet each list that can scroll to an item is asked to bring it in, as a swipe would. A tag
    // already composed is left alone, since searching a pager would page it away from the control
    // the screen shows first
    private fun scrollLazyListsTo(tag: String) {
        if (isComposed(tag)) return

        val lazyLists = compose.onAllNodes(hasScrollToNodeAction(), useUnmergedTree = true)

        for (index in lazyLists.fetchSemanticsNodes().indices) {
            runCatching { lazyLists[index].performScrollToNode(hasTestTag(tag)) }
            if (isComposed(tag)) break
        }
        compose.waitForIdle()
    }

    private fun isComposed(tag: String): Boolean = compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    // a section is where its content draws: a weighted slot's own bounds fill the space it was
    // given even when its content is small, and content that outgrows a fixed slot draws past them
    private fun sectionExtent(tag: String): VerticalExtent {
        val node = compose.onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode()
        val extent = node.children.map { it.drawnExtentPx() }.reduceOrNull(Rect::union) ?: node.ownBoundsPx()

        return extent.toLayoutDp()
    }

    private fun Rect.toLayoutDp(): VerticalExtent = with(layoutDensity) { VerticalExtent(top.toDp(), bottom.toDp()) }

    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0} at {1}x font")
        fun cases(): List<Array<Any>> =
            PinnedSectionsScreen.entries.flatMap { screen ->
                listOf(1f, LARGEST_SYSTEM_FONT_SCALE).map { fontScale -> arrayOf(screen, fontScale) }
            }
    }
}

private fun SemanticsNode.ownBoundsPx(): Rect = Rect(positionInRoot, size.toSize())

private fun SemanticsNode.drawnExtentPx(): Rect = children.fold(ownBoundsPx()) { extent, child -> extent.union(child.drawnExtentPx()) }

private fun Rect.union(other: Rect): Rect =
    Rect(
        left = minOf(left, other.left),
        top = minOf(top, other.top),
        right = maxOf(right, other.right),
        bottom = maxOf(bottom, other.bottom),
    )

private data class VerticalExtent(
    val top: Dp,
    val bottom: Dp,
)

/**
 * A screen whose sections spread over its height, the tags that mark each section from top to
 * bottom, the primary controls the page must scroll to whole, and how to render it
 */
enum class PinnedSectionsScreen(
    val sectionTags: List<String>,
    val onScreenTags: List<String>,
) {
    VERIFY_WORDS(listOf("verifyWords.topSection", "verifyWords.bottomSection"), listOf("verifyWords.skip")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val validator = WordValidator.preview(true, NumberOfBip39Words.TWELVE)
            val stateMachine = WordVerifyStateMachine(validator, 1u)
            closeables += stateMachine
            closeables += validator

            return {
                VerifyWordsScreen(
                    onBack = {},
                    onShowWords = {},
                    onSkip = {},
                    stateMachine = stateMachine,
                    snackbarHostState = remember { SnackbarHostState() },
                )
            }
        }
    },

    HOT_WALLET_CREATE(
        listOf("hotWalletCreate.topSection", "hotWalletCreate.bottomSection"),
        listOf("hotWalletCreate.primaryAction"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val manager = PendingWalletManager(NumberOfBip39Words.TWENTY_FOUR)
            closeables += manager

            return {
                HotWalletCreateScreen(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    snackbarHostState = remember { SnackbarHostState() },
                )
            }
        }
    },

    HOT_WALLET_IMPORT(
        listOf("hotWalletImport.topSection", "hotWalletImport.bottomSection"),
        listOf("hotWalletImport.import"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val manager = ImportWalletManager()
            closeables += manager

            return {
                HotWalletImportScreen(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    numberOfWords = NumberOfBip39Words.TWELVE,
                    importType = ImportType.MANUAL,
                    snackbarHostState = remember { SnackbarHostState() },
                    showNfcAction = false,
                )
            }
        }
    },

    // the screen focuses the amount field, so the emulator raises its keyboard, and the keyboard's
    // inset pads the screen as if this viewport reached the bottom of the window, leaving no
    // place to reach Next that a phone would have; the gap check still covers Next
    SEND_FLOW_SET_AMOUNT(listOf("sendFlowSetAmount.topSection", "sendFlow.next"), emptyList()) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val send = SendFixture.create(closeables)

            return {
                SendFlowSetAmountScreen(
                    app = send.app,
                    walletManager = send.walletManager,
                    sendFlowManager = send.sendFlowManager,
                    presenter = send.presenter,
                    onBack = {},
                    onNext = {},
                    onScanQr = {},
                    onChangeSpeed = {},
                )
            }
        }
    },

    COIN_CONTROL_SET_AMOUNT(listOf("coinControlSetAmount.topSection", "sendFlow.next"), listOf("sendFlow.next")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val send = SendFixture.create(closeables)

            return {
                CoinControlSetAmountScreen(
                    onBack = {},
                    onNext = {},
                    onScanQr = {},
                    onChangeSpeed = {},
                    onAmountTap = {},
                    onUtxoDetailsClick = {},
                    balanceAmount = "1,234,567",
                    balanceDenomination = "sats",
                    sendingAmount = "1,234,567",
                    sendingDenomination = "sats",
                    dollarEquivalentText = "$1,234.56",
                    initialAddress = "",
                    feeEta = "30 minutes",
                    feeAmount = "1,234 sats",
                    totalSpendingCrypto = "1,235,801 sats",
                    totalSpendingFiat = "$1,235.79",
                    utxoCount = 3,
                    utxos = emptyList(),
                    app = send.app,
                    sendFlowManager = send.sendFlowManager,
                    walletManager = send.walletManager,
                    presenter = send.presenter,
                    onAddressChanged = {},
                )
            }
        }
    },

    SEND_FLOW_CONFIRM(
        listOf("sendFlowConfirm.topSection", "sendFlowConfirm.bottomSection"),
        listOf("sendFlowConfirm.swipeToSend"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val send = SendFixture.create(closeables)
            val details = confirmDetailsPreviewNew()

            return {
                SendFlowConfirmScreen(
                    app = send.app,
                    walletManager = send.walletManager,
                    details = details,
                    sendState = SendState.Idle,
                    payjoinWaiting = null,
                    onSendWithoutPayjoin = {},
                    onBack = {},
                    onSwipeToSend = {},
                )
            }
        }
    },

    TRANSACTION_DETAILS(
        listOf("transactionDetails.topSection", "transactionDetails.bottomSection"),
        listOf("transactionDetails.showDetails"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val walletManager = WalletManager.previewNew()
            val presentation = TransactionDetailsPresentation.previewConfirmedSent()
            closeables += presentation
            closeables += walletManager

            return {
                TransactionDetailsScreen(
                    app = remember { AppManager.getInstance() },
                    manager = walletManager,
                    presentation = presentation,
                    txId = presentation.txId(),
                    refreshOnAppear = false,
                )
            }
        }
    },

    VERIFICATION_COMPLETE(
        listOf("verificationComplete.icon", "verificationComplete.bottomSection"),
        onScreenTags = listOf("verificationComplete.goToWallet"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                VerificationCompleteScreen(
                    app = remember { AppManager.getInstance() },
                    manager = null,
                    snackbarHostState = remember { SnackbarHostState() },
                )
            }
    },

    // the PIN setup and change screens show a cancel header above the title, the lock screen doesn't
    NUMBER_PAD_PIN(
        listOf("numberPadPin.header", "numberPadPin.title", "numberPadPin.pinBoxes", "numberPadPin.keypad"),
        onScreenTags = listOf("numberPadPin.digit.1", "numberPadPin.digit.0", "numberPadPin.delete"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                NumberPadPinView(
                    title = "Enter Current PIN",
                    isPinCorrect = { false },
                    backAction = {},
                )
            }
    },

    LOCK_SCREEN_PIN(
        listOf("numberPadPin.title", "numberPadPin.pinBoxes", "numberPadPin.keypad"),
        onScreenTags = listOf("numberPadPin.digit.1", "numberPadPin.digit.0", "numberPadPin.delete"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                NumberPadPinView(isPinCorrect = { false })
            }
    },

    CLOUD_BACKUP_ENABLED(
        listOf(
            "onboarding.cloudBackup.success.hero",
            "onboarding.cloudBackup.success.message",
            "onboarding.cloudBackup.success.continue",
        ),
        listOf("onboarding.cloudBackup.success.continue"),
    ) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                OnboardingCloudBackupSuccessView(onContinue = {})
            }
    },

    TAP_SIGNER_CHOOSE_CHAIN_CODE(tapSignerSheetSections("tapSignerChoose", "tapSignerChoose.advancedSetup"), listOf("tapSignerChoose.advancedSetup")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val tapSigner = previewTapSigner(closeables)
            val manager = tapSignerManager(TapSignerRoute.InitSelect(tapSigner), closeables)

            return {
                TapSignerChooseChainCode(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    tapSigner = tapSigner,
                )
            }
        }
    },

    TAP_SIGNER_ADVANCED_CHAIN_CODE(tapSignerSheetSections("tapSignerAdvanced", "tapSignerAdvanced.continue"), listOf("tapSignerAdvanced.continue")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val tapSigner = previewTapSigner(closeables)
            val manager = tapSignerManager(TapSignerRoute.InitAdvanced(tapSigner), closeables)

            return {
                TapSignerAdvancedChainCode(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    tapSigner = tapSigner,
                )
            }
        }
    },

    TAP_SIGNER_SETUP_SUCCESS(tapSignerSheetSections("tapSignerSetupSuccess", "tapSignerSetupSuccess.continue"), listOf("tapSignerSetupSuccess.continue")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                TapSignerSetupSuccessContent(onCancel = {}, onDownloadBackup = {}, onContinue = {})
            }
    },

    TAP_SIGNER_SETUP_RETRY(tapSignerSheetSections("tapSignerSetupRetry", "tapSignerSetupRetry.continue"), listOf("tapSignerSetupRetry.continue")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val tapSigner = previewTapSigner(closeables)
            val response = tapSignerSetupRetryContinueCmd(true)
            val manager = tapSignerManager(TapSignerRoute.SetupRetry(tapSigner, response), closeables)

            return {
                TapSignerSetupRetryView(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    tapSigner = tapSigner,
                    response = response,
                )
            }
        }
    },

    TAP_SIGNER_IMPORT_SUCCESS(tapSignerSheetSections("tapSignerImportSuccess", "tapSignerImportSuccess.continue"), listOf("tapSignerImportSuccess.continue")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit =
            {
                TapSignerImportSuccessContent(
                    saveState = TapSignerImportSaveState.Saved("preview-wallet"),
                    onCancel = {},
                    onRetry = {},
                    onContinue = {},
                )
            }
    },

    TAP_SIGNER_IMPORT_RETRY(tapSignerSheetSections("tapSignerImportRetry", "tapSignerImportRetry.retry"), listOf("tapSignerImportRetry.retry")) {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val tapSigner = previewTapSigner(closeables)
            val manager = tapSignerManager(TapSignerRoute.ImportRetry(tapSigner), closeables)

            return {
                TapSignerImportRetryView(
                    app = remember { AppManager.getInstance() },
                    manager = manager,
                    tapSigner = tapSigner,
                )
            }
        }
    },
    ;

    /** Makes the screen's fixtures, adding what needs closing to [closeables], and returns its content */
    abstract fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit
}

/** The managers a send flow screen reads, made from a preview wallet */
private class SendFixture(
    val app: AppManager,
    val walletManager: WalletManager,
    val sendFlowManager: SendFlowManager,
    val presenter: SendFlowPresenter,
) {
    companion object {
        fun create(closeables: MutableList<AutoCloseable>): SendFixture {
            val app = AppManager.getInstance()
            val walletManager = WalletManager.previewNew()
            val presenter = SendFlowPresenter(app, walletManager)
            val sendFlowManager =
                SendFlowManager(
                    walletManager.newSendFlowManager(walletManager.balance),
                    presenter,
                )

            // the send flow manager reads the wallet manager, so it closes first
            closeables += sendFlowManager
            closeables += walletManager

            return SendFixture(app, walletManager, sendFlowManager, presenter)
        }
    }
}

/** The header, content and action of a TapSigner sheet, tagged under [prefix] */
private fun tapSignerSheetSections(
    prefix: String,
    actionTag: String,
): List<String> = listOf("$prefix.header", "$prefix.content", actionTag)

private fun previewTapSigner(closeables: MutableList<AutoCloseable>): TapSigner =
    tapSignerPreviewNew(true).also { closeables += it }

private fun tapSignerManager(
    route: TapSignerRoute,
    closeables: MutableList<AutoCloseable>,
): TapSignerManager {
    val manager = TapSignerManager(route)

    // the manager reads the TapSigner, so it closes first
    closeables.add(0, AutoCloseable { manager.close() })
    return manager
}
