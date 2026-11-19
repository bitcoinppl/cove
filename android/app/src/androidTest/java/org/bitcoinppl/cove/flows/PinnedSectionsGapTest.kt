package org.bitcoinppl.cove.flows

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.PendingWalletManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletCreateScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletImportScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.VerifyWordsScreen
import org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails.TransactionDetailsScreen
import org.bitcoinppl.cove.flows.SendFlow.ConfirmScreen.SendFlowConfirmScreen
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.flows.SendFlow.SendState
import org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen.CoinControlSetAmountScreen
import org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen.SendFlowSetAmountScreen
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.WithFontScale
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveAndroidComposeRule
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.TransactionDetailsPresentation
import org.bitcoinppl.cove_core.WordValidator
import org.bitcoinppl.cove_core.WordVerifyStateMachine
import org.bitcoinppl.cove_core.types.confirmDetailsPreviewNew
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

// a Galaxy S20 at its default display size is about 360dp wide, and 640dp tall is a short phone
private val VIEWPORT_WIDTH = 360.dp
private val VIEWPORT_HEIGHT = 640.dp

// matches the spacing the screens keep between their pinned sections, as iOS does with VStack spacing
private val MIN_SECTION_GAP = 24.dp

// layout rounds each position to a whole pixel, so a gap can land a fraction of a dp short
private val GAP_ROUNDING_TOLERANCE = 0.5.dp

/**
 * Every screen that pins one section to the top and another to the bottom of a scrolling column
 * must keep them apart once its content outgrows a compact viewport, at the default and the
 * largest font scale
 *
 * Spreading only the leftover space between the sections, as `Arrangement.SpaceBetween` or a
 * weighted spacer does, leaves no gap once nothing is left over, so the sections touch on a
 * small screen or at a large display or font size, as the Verify Recovery Words dots did against
 * the word chips. A check that each control fits the viewport misses that. Add a screen with
 * this layout to [PinnedSectionsScreen]
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
                WithFontScale(fontScale) {
                    Box(Modifier.size(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)) {
                        content()
                    }
                }
            }
        }
        compose.waitForIdle()

        // unclipped bounds measure a section the scroll has moved out of view too
        val upper = compose.onNodeWithTag(screen.upperTag).getUnclippedBoundsInRoot()
        val lower = compose.onNodeWithTag(screen.lowerTag).getUnclippedBoundsInRoot()
        val gap = lower.top - upper.bottom

        assertTrue(
            "${screen.lowerTag} should start at least $MIN_SECTION_GAP below ${screen.upperTag} " +
                "at ${fontScale}x font, but the gap is $gap",
            gap >= MIN_SECTION_GAP - GAP_ROUNDING_TOLERANCE,
        )
    }

    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0} at {1}x font")
        fun cases(): List<Array<Any>> =
            PinnedSectionsScreen.entries.flatMap { screen ->
                listOf(1f, LARGEST_SYSTEM_FONT_SCALE).map { fontScale -> arrayOf(screen, fontScale) }
            }
    }
}

/**
 * A screen with a top and a bottom section pinned apart in a scrolling column, the tags that
 * mark each section, and how to render it
 */
enum class PinnedSectionsScreen(
    val upperTag: String,
    val lowerTag: String,
) {
    VERIFY_WORDS("verifyWords.topSection", "verifyWords.bottomSection") {
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

    HOT_WALLET_CREATE("hotWalletCreate.topSection", "hotWalletCreate.bottomSection") {
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

    HOT_WALLET_IMPORT("hotWalletImport.topSection", "hotWalletImport.bottomSection") {
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

    SEND_FLOW_SET_AMOUNT("sendFlowSetAmount.topSection", "sendFlow.next") {
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

    COIN_CONTROL_SET_AMOUNT("coinControlSetAmount.topSection", "sendFlow.next") {
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

    SEND_FLOW_CONFIRM("sendFlowConfirm.topSection", "sendFlowConfirm.bottomSection") {
        override fun setUp(closeables: MutableList<AutoCloseable>): @Composable () -> Unit {
            val send = SendFixture.create(closeables)
            val details = confirmDetailsPreviewNew()

            return {
                SendFlowConfirmScreen(
                    app = send.app,
                    walletManager = send.walletManager,
                    sendFlowManager = send.sendFlowManager,
                    details = details,
                    sendState = SendState.Idle,
                    onBack = {},
                    onSwipeToSend = {},
                )
            }
        }
    },

    TRANSACTION_DETAILS("transactionDetails.topSection", "transactionDetails.bottomSection") {
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
