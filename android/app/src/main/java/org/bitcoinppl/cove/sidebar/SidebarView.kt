package org.bitcoinppl.cove.sidebar

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletMetadata
import android.util.Log
import androidx.compose.foundation.Image

@Composable
fun SidebarView(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    var walletList by remember { mutableStateOf(app.wallets) }
    var draggedWalletId by remember { mutableStateOf<String?>(null) }
    var draggedDistance by remember { mutableFloatStateOf(0f) }
    var dragStartCenterY by remember { mutableFloatStateOf(0f) }
    val listState = rememberLazyListState()
    val haptic = LocalHapticFeedback.current

    LaunchedEffect(app.wallets, draggedWalletId) {
        // Keep local list in sync with source-of-truth while not actively dragging.
        if (draggedWalletId == null) {
            walletList = app.wallets
        }
    }

    Column(
        modifier =
            modifier
                .width(280.dp)
                .fillMaxHeight()
                .background(CoveColor.midnightBlue)
                .padding(WindowInsets.safeDrawing.asPaddingValues())
                .padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(0.dp),
    ) {
        // header with icon and NFC button
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Image(
                painter = painterResource(id = R.drawable.cove_logo),
                contentDescription = "Cove",
                modifier =
                    Modifier
                        .size(65.dp)
                        .clip(CircleShape),
            )

            IconButton(
                onClick = {
                    app.closeSidebarAndNavigate {
                        app.scanNfc()
                    }
                },
            ) {
                Icon(
                    imageVector = Icons.Default.Nfc,
                    contentDescription = "NFC Scan",
                    tint = Color.White,
                    modifier = Modifier.size(24.dp),
                )
            }
        }

        Spacer(modifier = Modifier.height(22.dp))

        HorizontalDivider(
            color = Color.White.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(22.dp))

        // my wallets header
        Text(
            text = "My Wallets",
            color = Color.White,
            fontSize = 17.sp,
            fontWeight = FontWeight.Medium,
        )

        Spacer(modifier = Modifier.height(12.dp))

        // wallet list
        LazyColumn(
            modifier = Modifier.weight(1f),
            state = listState,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            itemsIndexed(
                items = walletList,
                key = { _, wallet -> wallet.id.toString() },
            ) { _, wallet ->
                val isDragged = wallet.id == draggedWalletId
                WalletItem(
                    wallet = wallet,
                    modifier =
                        Modifier
                            .graphicsLayer {
                                translationY = if (isDragged) draggedDistance else 0f
                            }.pointerInput(wallet.id, walletList) {
                                detectDragGesturesAfterLongPress(
                                    onDragStart = {
                                        draggedWalletId = wallet.id
                                        draggedDistance = 0f
                                        val itemInfo =
                                            listState.layoutInfo.visibleItemsInfo.firstOrNull {
                                                it.key == wallet.id.toString()
                                            }
                                        dragStartCenterY =
                                            if (itemInfo != null) {
                                                itemInfo.offset + (itemInfo.size / 2f)
                                            } else {
                                                0f
                                            }
                                        haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                                    },
                                    onDrag = { change, dragAmount ->
                                        change.consume()
                                        val draggedId = draggedWalletId ?: return@detectDragGesturesAfterLongPress
                                        draggedDistance += dragAmount.y
                                        val fromIndex = walletList.indexOfFirst { it.id == draggedId }
                                        if (fromIndex == -1) return@detectDragGesturesAfterLongPress

                                        val currentCenterY = dragStartCenterY + draggedDistance
                                        val targetInfo =
                                            listState.layoutInfo.visibleItemsInfo.firstOrNull { info ->
                                                currentCenterY >= info.offset &&
                                                    currentCenterY <= info.offset + info.size
                                            }
                                                ?: return@detectDragGesturesAfterLongPress

                                        val toIndex = targetInfo.index
                                        if (toIndex == fromIndex) return@detectDragGesturesAfterLongPress
                                        if (toIndex !in walletList.indices) return@detectDragGesturesAfterLongPress

                                        walletList = walletList.move(fromIndex, toIndex)
                                        val refreshedInfo =
                                            listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == toIndex }
                                        if (refreshedInfo != null) {
                                            dragStartCenterY = refreshedInfo.offset + (refreshedInfo.size / 2f)
                                            draggedDistance = currentCenterY - dragStartCenterY
                                        } else {
                                            draggedDistance = 0f
                                        }
                                    },
                                    onDragEnd = {
                                        val draggedId = draggedWalletId
                                        if (draggedId != null) {
                                            val appOrder = app.wallets.map { it.id }
                                            val localOrder = walletList.map { it.id }
                                            if (localOrder != appOrder) {
                                                runCatching {
                                                    app.database.wallets().reorderWallets(orderedIds = localOrder)
                                                }.onFailure {
                                                    Log.e("SidebarView", "Failed to reorder wallets", it)
                                                    walletList = app.wallets
                                                }
                                            }
                                        }
                                        draggedWalletId = null
                                        draggedDistance = 0f
                                        dragStartCenterY = 0f
                                        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                    },
                                    onDragCancel = {
                                        draggedWalletId = null
                                        draggedDistance = 0f
                                        dragStartCenterY = 0f
                                        walletList = app.wallets
                                    },
                                )
                            },
                    onClick = {
                        if (draggedWalletId != null) return@WalletItem
                        app.closeSidebarAndNavigate {
                            app.rust.selectWallet(wallet.id)
                        }
                    },
                )
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        HorizontalDivider(
            color = CoveColor.coveLightGray.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(32.dp))

        // add wallet button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.closeSidebarAndNavigate {
                            if (app.wallets.isEmpty()) {
                                app.resetRoute(RouteFactory().newWalletSelect())
                            } else {
                                app.pushRoute(RouteFactory().newWalletSelect())
                            }
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Add,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Add Wallet",
                color = Color.White,
                fontSize = 17.sp,
            )
        }

        Spacer(modifier = Modifier.height(22.dp))

        // settings button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.closeSidebarAndNavigate {
                            app.pushRoute(Route.Settings(SettingsRoute.Main))
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Settings,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Settings",
                color = Color.White,
                fontSize = 17.sp,
            )
        }
    }
}

@Composable
private fun WalletItem(
    wallet: WalletMetadata,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .background(CoveColor.coveLightGray.copy(alpha = 0.06f))
                .clickable(onClick = onClick)
                .padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // color indicator
        Box(
            modifier =
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(wallet.color.toComposeColor()),
        )

        // wallet name
        AutoSizeText(
            text = wallet.name ?: "Wallet",
            color = Color.White,
            maxFontSize = 17.sp,
            minimumScaleFactor = 0.80f,
            modifier = Modifier.weight(1f),
        )
    }
}

private fun List<WalletMetadata>.move(
    fromIndex: Int,
    toIndex: Int,
): List<WalletMetadata> {
    if (fromIndex == toIndex) return this
    val mutable = toMutableList()
    val item = mutable.removeAt(fromIndex)
    val safeTarget = toIndex.coerceIn(0, mutable.size)
    mutable.add(safeTarget, item)
    return mutable.toList()
}

// convert wallet color to compose color
private fun WalletColor.toComposeColor(): Color =
    when (this) {
        is WalletColor.Red -> CoveColor.pastelRed
        is WalletColor.Blue -> CoveColor.pastelBlue
        is WalletColor.Green -> CoveColor.walletGreen
        is WalletColor.Yellow -> CoveColor.pastelYellow
        is WalletColor.Orange -> CoveColor.beige
        is WalletColor.Purple -> CoveColor.walletColorPurple
        is WalletColor.Pink -> CoveColor.walletColorLightRed
        is WalletColor.CoolGray -> CoveColor.almostGray
        is WalletColor.WBeige -> CoveColor.beige
        is WalletColor.WPastelBlue -> CoveColor.pastelBlue
        is WalletColor.WPastelNavy -> CoveColor.pastelNavy
        is WalletColor.WPastelRed -> CoveColor.pastelRed
        is WalletColor.WPastelYellow -> CoveColor.pastelYellow
        is WalletColor.WLightMint -> CoveColor.lightMint
        is WalletColor.WPastelTeal -> CoveColor.pastelTeal
        is WalletColor.WLightPastelYellow -> CoveColor.lightPastelYellow
        is WalletColor.WAlmostGray -> CoveColor.almostGray
        is WalletColor.WAlmostWhite -> CoveColor.almostWhite
        is WalletColor.Custom -> Color.Gray
    }
