package org.bitcoinppl.cove.flows

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.NewWalletFlow.NewWalletSelectScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletImportScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletSelectScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.VerificationCompleteScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.VerifyWordsScreen
import org.bitcoinppl.cove.flows.SendFlow.SendFlowConfirmScreen
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.flows.SendFlow.SendState
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerAdvancedChainCode
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerChooseChainCode
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerManager
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.saveNodeScreenshotToLayoutAudit
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.WordValidator
import org.bitcoinppl.cove_core.WordVerifyStateMachine
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.tapcard.tapSignerPreviewNew
import org.bitcoinppl.cove_core.types.confirmDetailsPreviewNew
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain

@LayoutRegressionTest
class AndroidCompactLayoutAuditTest {
    private val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(AndroidDeviceStayAwakeRule())
            .around(compose)

    private var importWalletManager: ImportWalletManager? = null
    private var wordValidator: WordValidator? = null
    private var wordVerifyStateMachine: WordVerifyStateMachine? = null
    private val sendFlowManagers = mutableListOf<SendFlowManager>()
    private val walletManagers = mutableListOf<WalletManager>()
    private val tapSignerManagers = mutableListOf<TapSignerManager>()
    private val tapSigners = mutableListOf<TapSigner>()

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @After
    fun closeManagers() {
        tapSignerManagers.forEach { it.close() }
        tapSignerManagers.clear()
        tapSigners.forEach { it.close() }
        tapSigners.clear()
        wordVerifyStateMachine?.close()
        wordVerifyStateMachine = null
        wordValidator?.close()
        wordValidator = null
        importWalletManager?.close()
        importWalletManager = null
        sendFlowManagers.forEach { it.close() }
        sendFlowManagers.clear()
        walletManagers.forEach { it.close() }
        walletManagers.clear()
    }

    @Test
    fun newWalletSelectCompactSnapshot() {
        val screenName = "new-wallet-select"

        renderCompact(screenName) {
            NewWalletSelectScreen(
                app = remember { AppManager.getInstance() },
                onBack = {},
                canGoBack = true,
                onOpenNewHotWallet = {},
                onOpenQrScan = {},
                onOpenNfcScan = {},
                snackbarHostState = remember { SnackbarHostState() },
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "newWalletSelect.hardwareWallet")
        assertNodeInsideViewport(screenName, "newWalletSelect.onThisDevice")
    }

    @Test
    fun hotWalletSelectCompactSnapshot() {
        val screenName = "hot-wallet-select"

        renderCompact(screenName) {
            HotWalletSelectScreen(
                app = remember { AppManager.getInstance() },
                snackbarHostState = remember { SnackbarHostState() },
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "hotWalletSelect.createWallet")
        assertNodeInsideViewport(screenName, "hotWalletSelect.importWallet")
    }

    @Test
    fun hotWalletImportCompactSnapshot() {
        val screenName = "hot-wallet-import"
        val manager = ImportWalletManager()
        importWalletManager = manager

        renderCompact(screenName) {
            HotWalletImportScreen(
                app = remember { AppManager.getInstance() },
                manager = manager,
                numberOfWords = NumberOfBip39Words.TWELVE,
                importType = ImportType.MANUAL,
                snackbarHostState = remember { SnackbarHostState() },
                showNfcAction = false,
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "hotWalletImport.import")
    }

    @Test
    fun verifyWordsCompactSnapshot() {
        val screenName = "verify-words"
        val validator = WordValidator.preview(true, NumberOfBip39Words.TWELVE)
        val stateMachine = WordVerifyStateMachine(validator, 1u)
        wordValidator = validator
        wordVerifyStateMachine = stateMachine

        renderCompact(screenName) {
            VerifyWordsScreen(
                onBack = {},
                onShowWords = {},
                onSkip = {},
                stateMachine = stateMachine,
                snackbarHostState = remember { SnackbarHostState() },
            )
        }

        assertNodeInsideViewport(screenName, "verifyWords.target", minBottomPadding = 0.dp)

        if (isAfterScreenshot()) {
            compose
                .onNodeWithTag("verifyWords.bottomPadding")
                .performScrollTo()
        }

        saveAuditScreenshot(screenName)

        compose
            .onNodeWithTag("verifyWords.bottomPadding")
            .performScrollTo()

        assertNodeInsideViewport(screenName, "verifyWords.showWords")
        assertNodeInsideViewport(screenName, "verifyWords.skip")
    }

    @Test
    fun verificationCompleteCompactSnapshot() {
        val screenName = "verification-complete"

        renderCompact(screenName) {
            VerificationCompleteScreen(
                app = remember { AppManager.getInstance() },
                manager = null,
                snackbarHostState = remember { SnackbarHostState() },
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "verificationComplete.goToWallet")
    }

    @Test
    fun tapSignerChooseChainCodeCompactSnapshot() {
        val screenName = "tap-signer-choose-chain-code"
        val tapSigner = previewTapSigner()
        val manager = TapSignerManager(TapSignerRoute.InitSelect(tapSigner))
        tapSignerManagers.add(manager)

        renderCompact(screenName, darkTheme = false) {
            TapSignerChooseChainCode(
                app = remember { AppManager.getInstance() },
                manager = manager,
                tapSigner = tapSigner,
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "tapSignerChoose.automaticSetup")
        assertNodeInsideViewport(screenName, "tapSignerChoose.advancedSetup")
    }

    @Test
    fun tapSignerAdvancedChainCodeCompactSnapshot() {
        val screenName = "tap-signer-advanced-chain-code"
        val tapSigner = previewTapSigner()
        val manager = TapSignerManager(TapSignerRoute.InitAdvanced(tapSigner))
        tapSignerManagers.add(manager)

        renderCompact(screenName, darkTheme = false) {
            TapSignerAdvancedChainCode(
                app = remember { AppManager.getInstance() },
                manager = manager,
                tapSigner = tapSigner,
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "tapSignerAdvanced.continue")
    }

    @Test
    fun numberPadPinCompactSnapshot() {
        val screenName = "number-pad-pin"

        renderCompact(screenName) {
            NumberPadPinView(
                title = "Enter Pin",
                isPinCorrect = { it == "123456" },
                backAction = {},
            )
        }

        saveBeforeScreenshot(screenName)
        assertNodeInsideViewport(screenName, "numberPadPin.digit.0")
        assertNodeInsideViewport(screenName, "numberPadPin.delete")
    }

    @Test
    fun sendFlowConfirmCompactSnapshot() {
        val screenName = "send-flow-confirm"
        val app = AppManager.getInstance()
        val walletManager = WalletManager.previewNew()
        walletManagers.add(walletManager)
        val sendFlowManager =
            SendFlowManager(
                walletManager.newSendFlowManager(walletManager.balance),
                SendFlowPresenter(app, walletManager),
            )
        sendFlowManagers.add(sendFlowManager)
        val details = confirmDetailsPreviewNew()

        renderCompact(screenName) {
            SendFlowConfirmScreen(
                app = app,
                walletManager = walletManager,
                sendFlowManager = sendFlowManager,
                details = details,
                sendState = SendState.Idle,
                onBack = {},
                onSwipeToSend = {},
            )
        }

        if (isAfterScreenshot()) {
            compose
                .onNodeWithTag("sendFlowConfirm.bottomPadding")
                .performScrollTo()
        }

        saveAuditScreenshot(screenName)

        compose
            .onNodeWithTag("sendFlowConfirm.bottomPadding")
            .performScrollTo()

        assertNodeInsideViewport(screenName, "sendFlowConfirm.swipeToSend")
    }

    private fun renderCompact(
        screenName: String,
        darkTheme: Boolean = true,
        content: @Composable () -> Unit,
    ) {
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.setContent {
            CoveTheme(darkTheme = darkTheme) {
                Box(
                    modifier =
                        Modifier
                            .width(auditViewportWidth())
                            .height(auditViewportHeight())
                            .testTag(viewportTag(screenName)),
                ) {
                    content()
                }
            }
        }
        compose.waitForIdle()
    }

    private fun saveBeforeScreenshot(screenName: String) {
        saveAuditScreenshot(screenName)
    }

    private fun saveAuditScreenshot(screenName: String) {
        compose.saveNodeScreenshotToLayoutAudit(
            tag = viewportTag(screenName),
            name = "$screenName-${screenshotSuffix()}.png",
        )
    }

    private fun screenshotSuffix(): String =
        InstrumentationRegistry
            .getArguments()
            .getString("layoutScreenshotSuffix")
            ?: "before"

    private fun isAfterScreenshot(): Boolean = screenshotSuffix().endsWith("after")

    private fun auditViewportWidth(): Dp =
        InstrumentationRegistry
            .getArguments()
            .getString("layoutViewportWidthDp")
            ?.toIntOrNull()
            ?.dp
            ?: 360.dp

    private fun auditViewportHeight(): Dp =
        InstrumentationRegistry
            .getArguments()
            .getString("layoutViewportHeightDp")
            ?.toIntOrNull()
            ?.dp
            ?: 640.dp

    private fun assertNodeInsideViewport(
        screenName: String,
        nodeTag: String,
        minBottomPadding: Dp = 16.dp,
    ) {
        compose
            .onNodeWithTag(nodeTag)
            .assertIsDisplayed()

        val viewportBounds =
            compose
                .onNodeWithTag(viewportTag(screenName))
                .getUnclippedBoundsInRoot()
        val nodeBounds =
            compose
                .onNodeWithTag(nodeTag)
                .getUnclippedBoundsInRoot()

        assertTrue(
            "$nodeTag should fit inside the compact viewport",
            nodeBounds.left >= viewportBounds.left &&
                nodeBounds.right <= viewportBounds.right &&
                nodeBounds.top >= viewportBounds.top &&
                nodeBounds.bottom <= viewportBounds.bottom - minBottomPadding,
        )
    }

    private fun viewportTag(screenName: String): String = "androidCompactAudit.$screenName.viewport"

    private fun previewTapSigner(): TapSigner =
        tapSignerPreviewNew(true).also { tapSigners.add(it) }
}
