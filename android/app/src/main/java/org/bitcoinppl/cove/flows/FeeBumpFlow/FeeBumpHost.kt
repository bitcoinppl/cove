@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.PendingSendStatus
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.types.TxId

/** The delayed send a "Taking longer than usual" popup is open for */
data class SpeedUpPopupTarget(
    val txId: TxId,
    val pendingSend: PendingSendStatus,
)

/**
 * A transaction whose long-press sheet is open
 *
 * @param openDetails opens its details the way tapping its row does
 */
class TransactionMenuTarget(
    val transaction: Transaction,
    val openDetails: () -> Unit,
)

/**
 * Which Speed Up window is open over a screen: the Speed Up sheet, the delayed popup or a
 * transaction's long-press sheet, at most one at a time
 *
 * [FeeBumpHost] makes one and provides it to the rows and details below as
 * [LocalFeeBumpPresenter]
 */
@Stable
class FeeBumpPresenter {
    var sheet by mutableStateOf<TxId?>(null)
        private set

    var popup by mutableStateOf<SpeedUpPopupTarget?>(null)
        private set

    var menu by mutableStateOf<TransactionMenuTarget?>(null)
        private set

    fun openSheet(txId: TxId) {
        closeAll()
        sheet = txId
    }

    fun openPopup(
        txId: TxId,
        pendingSend: PendingSendStatus,
    ) {
        closeAll()
        popup = SpeedUpPopupTarget(txId, pendingSend)
    }

    fun openMenu(target: TransactionMenuTarget) {
        closeAll()
        menu = target
    }

    fun closeAll() {
        sheet = null
        popup = null
        menu = null
    }
}

/** Opens Speed Up windows over the screen that hosts them, null outside a [FeeBumpHost] */
val LocalFeeBumpPresenter = staticCompositionLocalOf<FeeBumpPresenter?> { null }

/**
 * Hosts the Speed Up sheet, the delayed popup and the long-press sheet over [content]
 *
 * Each window makes its own managers when it opens and closes them when it goes. A window whose
 * Continue built a replacement closes before the confirm or export route is pushed
 *
 * @param onSheetClosed called once the Speed Up sheet closes without continuing, such as to
 *   refresh details that a send confirming meanwhile changed
 */
@Composable
fun FeeBumpHost(
    app: AppManager,
    walletManager: WalletManager,
    onSheetClosed: () -> Unit = {},
    content: @Composable () -> Unit,
) {
    val presenter = remember(walletManager) { FeeBumpPresenter() }

    fun push(route: Route) {
        presenter.closeAll()
        app.pushRoute(route)
    }

    CompositionLocalProvider(LocalFeeBumpPresenter provides presenter) {
        content()
    }

    presenter.sheet?.let { txId ->
        key(txId) {
            SpeedUpSheet(
                app = app,
                walletManager = walletManager,
                txId = txId,
                onReady = ::push,
                onDismiss = {
                    presenter.closeAll()
                    onSheetClosed()
                },
            )
        }
    }

    presenter.popup?.let { target ->
        key(target) {
            SpeedUpPopup(
                walletManager = walletManager,
                app = app,
                txId = target.txId,
                pendingSend = target.pendingSend,
                onReady = ::push,
                onCustomize = { presenter.openSheet(target.txId) },
                onDismiss = presenter::closeAll,
            )
        }
    }

    presenter.menu?.let { target ->
        key(target) {
            TransactionActionsSheet(
                app = app,
                walletManager = walletManager,
                target = target,
                onSpeedUp = presenter::openSheet,
                onDismiss = presenter::closeAll,
            )
        }
    }
}
