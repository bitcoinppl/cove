package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.material3.SnackbarHostState
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
import org.bitcoinppl.cove.PendingWalletManager
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.saveNodeScreenshotToLayoutAudit
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain

@LayoutRegressionTest
class HotWalletCreateScreenTest {
    private val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(AndroidDeviceStayAwakeRule())
            .around(compose)

    private var pendingWalletManager: PendingWalletManager? = null

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @After
    fun closePendingWalletManager() {
        pendingWalletManager?.close()
        pendingWalletManager = null
    }

    @Test
    fun primaryActionFitsCompactViewportWithBottomPadding() {
        val manager = PendingWalletManager(NumberOfBip39Words.TWELVE)
        pendingWalletManager = manager

        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.setContent {
            CoveTheme(darkTheme = true) {
                Box(
                    modifier =
                        Modifier
                            .width(auditViewportWidth())
                            .height(auditViewportHeight())
                            .testTag("hotWalletCreate.viewport"),
                ) {
                    HotWalletCreateScreen(
                        app = remember { AppManager.getInstance() },
                        manager = manager,
                        snackbarHostState = remember { SnackbarHostState() },
                    )
                }
            }
        }

        if (screenshotMode() == "initial") {
            saveViewportScreenshot()
            return
        }

        compose
            .onNodeWithTag("hotWalletCreate.bottomPadding")
            .performScrollTo()

        compose
            .onNodeWithTag("hotWalletCreate.primaryAction")
            .assertIsDisplayed()

        if (screenshotMode() == "after-scroll") {
            saveViewportScreenshot()
        }

        val viewportBounds =
            compose
                .onNodeWithTag("hotWalletCreate.viewport")
                .getUnclippedBoundsInRoot()
        val actionBounds =
            compose
                .onNodeWithTag("hotWalletCreate.primaryAction")
                .getUnclippedBoundsInRoot()
        val minimumBottomPadding = 40.dp

        assertTrue(
            "primary action should fit within compact viewport with bottom padding after scrolling",
            actionBounds.left >= viewportBounds.left &&
                actionBounds.right <= viewportBounds.right &&
                actionBounds.top >= viewportBounds.top &&
                actionBounds.bottom <= viewportBounds.bottom - minimumBottomPadding,
        )
    }

    private fun screenshotMode(): String? =
        InstrumentationRegistry
            .getArguments()
            .getString("layoutScreenshotMode")

    private fun saveViewportScreenshot() {
        val name =
            InstrumentationRegistry
                .getArguments()
                .getString("layoutScreenshotName")
                ?: return
        compose.saveNodeScreenshotToLayoutAudit("hotWalletCreate.viewport", name)
    }

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
}
